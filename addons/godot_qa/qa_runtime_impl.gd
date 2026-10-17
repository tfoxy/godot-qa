extends RefCounted

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
var _assertions: RefCounted = null


func _init(scene_root: Node, root_window: Window, runtime_errors := [], held_actions := {}) -> void:
	_scene_root = scene_root
	_root_window = root_window
	_runtime_errors = runtime_errors if runtime_errors is Array else []
	_held_actions = held_actions if held_actions is Dictionary else {}
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
			return await _press_key(str(command_params.get("key", "")))
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
	var raw_targets: Array = params.get("targets", []) if params.get("targets", []) is Array else []
	if raw_targets.is_empty():
		return _error(
			"invalid_params",
			"visual.audit_targets requires at least one target",
			{
				"command": "visual.audit_targets",
				"field": "targets",
				"expected": "non-empty target locator array",
				"actual": params.get("targets", null),
			},
		)

	var include_state := bool(params.get("include_state", false))
	var requested_state_methods: Array = []
	if params.has("state_methods"):
		if not (params.get("state_methods") is Array):
			return _error(
				"invalid_params",
				"visual.audit_targets state_methods must be an array",
				{
					"command": "visual.audit_targets",
					"field": "state_methods",
					"expected": "array of approved read-only visual state method names",
					"actual": params.get("state_methods", null),
				},
			)
		for raw_method in params.get("state_methods"):
			var method := str(raw_method).strip_edges()
			if not _approved_read_only_state_method(method):
				return _error(
					"invalid_params",
					"visual.audit_targets state method is not approved",
					{
						"command": "visual.audit_targets",
						"field": "state_methods",
						"expected": ["get_qa_visual_state", "get_visual_qa_state", "get_qa_readability_state"],
						"actual": method,
					},
				)
			if not requested_state_methods.has(method):
				requested_state_methods.append(method)
		include_state = true

	var viewport := viewport_rect()
	var targets: Array = []
	for raw_target in raw_targets:
		var locator := str(raw_target).strip_edges()
		if locator == "":
			targets.append({
				"locator": locator,
				"resolved": "",
				"visible": false,
				"viewport": _visual_audit_viewport_metadata(viewport),
				"visual_evidence": QaSnapshot.visual_evidence_for_node(null),
				"error": _error_payload(
					"invalid_target",
					"visual.audit_targets target locator must be non-empty",
					{"expected": "non-empty target locator", "actual": raw_target},
				),
			})
			continue
		var resolution := resolve_target(locator)
		if not bool(resolution.get("ok", false)):
			targets.append({
				"locator": locator,
				"resolved": str(resolution.get("resolved", "")),
				"visible": false,
				"viewport": _visual_audit_viewport_metadata(viewport),
				"visual_evidence": QaSnapshot.visual_evidence_for_node(null),
				"error": (resolution.get("error", {}) as Dictionary).duplicate(true),
			})
			continue
		var node := resolution.get("node") as Node
		var evidence := QaSnapshot.visual_evidence_for_node(node)
		var target_payload := {
			"locator": locator,
			"resolved": str(resolution.get("resolved", "")),
			"path": str(node.get_path()) if node != null else "",
			"visible": is_target_visible(node),
			"viewport": _visual_audit_viewport_metadata(viewport),
			"visual_evidence": evidence,
			"inside_viewport": _visual_evidence_inside_viewport(evidence, viewport),
		}
		if include_state:
			target_payload["state"] = _visual_audit_state_for_node(node, requested_state_methods)
		targets.append(target_payload)

	return _ok({
		"targets": targets,
		"target_count": targets.size(),
		"viewport": _visual_audit_viewport_metadata(viewport),
	})


func _visual_audit_state_for_node(node: Node, requested_methods: Array) -> Dictionary:
	var state := {
		"available": false,
		"methods": {},
	}
	if node == null:
		return state
	var methods := requested_methods.duplicate()
	if methods.is_empty():
		methods = ["get_qa_visual_state", "get_visual_qa_state", "get_qa_readability_state"]
	for raw_method in methods:
		var method := str(raw_method).strip_edges()
		if not _approved_read_only_state_method(method) or not node.has_method(method):
			continue
		if not _method_has_zero_required_arguments(node, method):
			(state["methods"] as Dictionary)[method] = {
				"available": false,
				"unsupported_reason": "method_requires_arguments",
			}
			continue
		var value = node.call(method)
		(state["methods"] as Dictionary)[method] = {
			"available": true,
			"value": _json_compatible_value(value),
		}
		state["available"] = true
	return state


func _approved_read_only_state_method(method: String) -> bool:
	return method == "get_qa_visual_state" \
		or method == "get_visual_qa_state" \
		or method == "get_qa_readability_state"


func _visual_audit_viewport_metadata(viewport: Rect2) -> Dictionary:
	return {
		"rect": _rect2_payload(viewport),
		"size": _vector2_payload(viewport.size),
		"coordinate_space": "viewport_2d",
	}


func _visual_evidence_inside_viewport(evidence: Dictionary, viewport: Rect2) -> Variant:
	var raw_rect = evidence.get("rect", null)
	if raw_rect == null:
		return null
	var rect := _rect2_from_visual_evidence(raw_rect)
	if rect == null:
		return null
	if rect.size.x <= 0.0 or rect.size.y <= 0.0 or viewport.size.x <= 0.0 or viewport.size.y <= 0.0:
		return false
	return viewport.intersects(rect, true)


func _rect2_from_visual_evidence(raw_rect: Variant) -> Variant:
	if raw_rect is Array:
		var values: Array = raw_rect
		if values.size() >= 4 and _is_number(values[0]) and _is_number(values[1]) and _is_number(values[2]) and _is_number(values[3]):
			return Rect2(Vector2(float(values[0]), float(values[1])), Vector2(float(values[2]), float(values[3])))
	if raw_rect is Dictionary:
		var dictionary: Dictionary = raw_rect
		if _is_number(dictionary.get("x")) and _is_number(dictionary.get("y")) and _is_number(dictionary.get("width")) and _is_number(dictionary.get("height")):
			return Rect2(Vector2(float(dictionary.get("x")), float(dictionary.get("y"))), Vector2(float(dictionary.get("width")), float(dictionary.get("height"))))
	return null


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
	return _resolve_setup_point_provider(point_request, offset)


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
	return _register_provider(_state_providers, "state_provider", provider_id, owner, method_name, metadata, 0, true)


static func unregister_state_provider(provider_id: String) -> Dictionary:
	return _unregister_provider(_state_providers, "state_provider", provider_id)


static func register_probe_provider(provider_id: String, owner: Object, method_name: String, metadata: Dictionary = {}) -> Dictionary:
	return _register_provider(_probe_providers, "probe_provider", provider_id, owner, method_name, metadata, 1, true)


static func unregister_probe_provider(provider_id: String) -> Dictionary:
	return _unregister_provider(_probe_providers, "probe_provider", provider_id)


static func register_fixture_driver(driver_id: String, owner: Object, method_name: String, metadata: Dictionary = {}) -> Dictionary:
	return _register_provider(_fixture_drivers, "fixture_driver", driver_id, owner, method_name, metadata, 1, false)


static func unregister_fixture_driver(driver_id: String) -> Dictionary:
	return _unregister_provider(_fixture_drivers, "fixture_driver", driver_id)


# Policy providers compute desired joypad inputs for drive_policy steps. They
# are read-only consumers: the method receives one request dictionary and must
# return {"inputs": [...]} without mutating game state, so mutating method
# names are rejected exactly like state providers.
static func register_policy_provider(provider_id: String, owner: Object, method_name: String, metadata: Dictionary = {}) -> Dictionary:
	return _register_provider(_policy_providers, "policy_provider", provider_id, owner, method_name, metadata, 1, true)


static func unregister_policy_provider(provider_id: String) -> Dictionary:
	return _unregister_provider(_policy_providers, "policy_provider", provider_id)


static func clear_provider_registry() -> void:
	_state_providers.clear()
	_probe_providers.clear()
	_fixture_drivers.clear()
	_policy_providers.clear()


static func list_state_providers() -> Array[Dictionary]:
	return _list_providers(_state_providers)


static func list_probe_providers() -> Array[Dictionary]:
	return _list_providers(_probe_providers)


static func list_fixture_drivers() -> Array[Dictionary]:
	return _list_providers(_fixture_drivers)


static func list_policy_providers() -> Array[Dictionary]:
	return _list_providers(_policy_providers)


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


static func _register_provider(registry: Dictionary, provider_kind: String, provider_id: String, owner: Object, method_name: String, metadata: Dictionary, argument_count: int, reject_mutating_name: bool) -> Dictionary:
	var normalized_id := str(provider_id).strip_edges()
	var normalized_method := str(method_name).strip_edges()
	if normalized_id == "" or not _provider_id_is_valid(normalized_id):
		return _static_error("invalid_provider_id", "Provider id must be non-empty and use only ASCII letters, numbers, '.', '_', ':', or '-'", {"provider_kind": provider_kind, "provider_id": provider_id})
	if registry.has(normalized_id):
		return _static_error("provider_already_registered", "Provider id is already registered", {"provider_kind": provider_kind, "provider_id": normalized_id})
	if owner == null or not is_instance_valid(owner):
		return _static_error("provider_owner_invalid", "Provider owner must be a valid Object", {"provider_kind": provider_kind, "provider_id": normalized_id})
	if normalized_method == "" or not owner.has_method(normalized_method):
		return _static_error("provider_method_missing", "Provider method was not found on the owner", {"provider_kind": provider_kind, "provider_id": normalized_id, "method": normalized_method})
	if reject_mutating_name and _provider_method_name_is_mutating(normalized_method):
		return _static_error("provider_method_mutating", "Read-only providers cannot register mutating method names", {"provider_kind": provider_kind, "provider_id": normalized_id, "method": normalized_method})
	var reflected_count := _registered_method_argument_count(owner, normalized_method)
	if reflected_count != argument_count:
		return _static_error("provider_method_arguments", "Provider method has an unsupported argument count", {"provider_kind": provider_kind, "provider_id": normalized_id, "method": normalized_method, "expected": argument_count, "actual": reflected_count})
	if not _provider_registry_json_compatible(metadata):
		return _static_error("provider_metadata_invalid", "Provider metadata must be JSON-compatible", {"provider_kind": provider_kind, "provider_id": normalized_id})
	registry[normalized_id] = {
		"id": normalized_id,
		"kind": provider_kind,
		"owner": owner,
		"method": normalized_method,
		"metadata": metadata.duplicate(true),
	}
	return {"ok": true, "data": _provider_public_entry(registry[normalized_id])}


static func _unregister_provider(registry: Dictionary, provider_kind: String, provider_id: String) -> Dictionary:
	var normalized_id := str(provider_id).strip_edges()
	if normalized_id == "" or not registry.has(normalized_id):
		return _static_error("provider_not_registered", "Provider id is not registered", {"provider_kind": provider_kind, "provider_id": normalized_id})
	var entry := _provider_public_entry(registry.get(normalized_id, {}))
	registry.erase(normalized_id)
	return {"ok": true, "data": entry}


static func _list_providers(registry: Dictionary) -> Array[Dictionary]:
	var ids: Array[String] = []
	for provider_id in registry.keys():
		ids.append(str(provider_id))
	ids.sort()
	var providers: Array[Dictionary] = []
	for provider_id in ids:
		var entry = registry.get(provider_id, {})
		if entry is Dictionary:
			providers.append(_provider_public_entry(entry))
	return providers


static func _provider_public_entry(entry: Dictionary) -> Dictionary:
	var owner = entry.get("owner", null)
	var owner_path := ""
	var owner_class := ""
	if owner is Node:
		var owner_node := owner as Node
		owner_path = str(owner_node.get_path()) if owner_node.is_inside_tree() else ""
	if owner is Object:
		owner_class = (owner as Object).get_class()
	return {
		"id": str(entry.get("id", "")),
		"kind": str(entry.get("kind", "")),
		"method": str(entry.get("method", "")),
		"owner_class": owner_class,
		"owner_path": owner_path,
		"metadata": (entry.get("metadata", {}) as Dictionary).duplicate(true) if entry.get("metadata", {}) is Dictionary else {},
	}


static func _provider_id_is_valid(provider_id: String) -> bool:
	for character in provider_id:
		var codepoint := character.unicode_at(0)
		var is_digit := codepoint >= 48 and codepoint <= 57
		var is_upper := codepoint >= 65 and codepoint <= 90
		var is_lower := codepoint >= 97 and codepoint <= 122
		if not (is_digit or is_upper or is_lower or PROVIDER_ID_ALLOWED_EXTRA.contains(character)):
			return false
	return true


static func _provider_method_name_is_mutating(method_name: String) -> bool:
	var normalized := method_name.strip_edges()
	while normalized.begins_with("_"):
		normalized = normalized.substr(1)
	for prefix in PROVIDER_MUTATING_PREFIXES:
		if normalized.begins_with(prefix):
			return true
	return false


static func _registered_method_argument_count(owner: Object, method_name: String) -> int:
	for method_info in owner.get_method_list():
		if not method_info is Dictionary:
			continue
		var info: Dictionary = method_info
		if str(info.get("name", "")) != method_name:
			continue
		var args: Array = info.get("args", []) if info.get("args", []) is Array else []
		return args.size()
	return -1


static func _provider_registry_json_compatible(value: Variant) -> bool:
	match typeof(value):
		TYPE_NIL, TYPE_BOOL, TYPE_INT, TYPE_FLOAT, TYPE_STRING:
			return true
		TYPE_STRING_NAME, TYPE_NODE_PATH:
			return true
		TYPE_ARRAY:
			for item in value:
				if not _provider_registry_json_compatible(item):
					return false
			return true
		TYPE_DICTIONARY:
			var dictionary: Dictionary = value
			for key in dictionary.keys():
				if not _provider_registry_json_compatible(dictionary[key]):
					return false
			return true
		_:
			return false


static func _static_error(error_type: String, message: String, data: Dictionary) -> Dictionary:
	return {
		"ok": false,
		"error": {
			"type": error_type,
			"message": message,
			"data": data,
		},
	}


func _provider_entry(registry: Dictionary, provider_id: String, provider_kind: String) -> Dictionary:
	var normalized_id := str(provider_id).strip_edges()
	if normalized_id == "":
		return _error("provider_missing", "Provider id is required", {"provider_kind": provider_kind, "provider": provider_id})
	if not registry.has(normalized_id):
		return _error("provider_not_registered", "Provider id is not registered", {"provider_kind": provider_kind, "provider": normalized_id})
	var entry = registry.get(normalized_id, {})
	if not (entry is Dictionary):
		return _error("provider_invalid", "Provider registry entry is invalid", {"provider_kind": provider_kind, "provider": normalized_id})
	var provider_entry: Dictionary = entry
	var owner = provider_entry.get("owner", null)
	if owner == null or not is_instance_valid(owner):
		registry.erase(normalized_id)
		return _error("provider_owner_invalid", "Provider owner is no longer valid", {"provider_kind": provider_kind, "provider": normalized_id})
	var method_name := str(provider_entry.get("method", ""))
	if method_name == "" or not (owner as Object).has_method(method_name):
		return _error("provider_method_missing", "Provider method is no longer available", {"provider_kind": provider_kind, "provider": normalized_id, "method": method_name})
	return {"ok": true, "entry": provider_entry}


func describe_node(node: Variant) -> String:
	if node == null:
		return "none"
	if node is Node:
		var locator := stable_locator_for_node(node as Node)
		if locator != "":
			return locator
	return str(node)


func read_state_provider(provider_id: String) -> Dictionary:
	var provider := _provider_entry(_state_providers, provider_id, "state_provider")
	if not bool(provider.get("ok", false)):
		return provider
	var entry: Dictionary = provider.get("entry", {})
	var owner: Object = entry.get("owner")
	var method_name := str(entry.get("method", ""))
	var value = owner.call(method_name)
	if not _is_json_compatible_project_state_value(value):
		return _error(
			"invalid_state_source",
			"registered state provider returned non JSON-compatible data",
			{
				"provider": str(entry.get("id", "")),
				"method": method_name,
				"expected": "JSON-compatible Dictionary or Array",
				"actual_type": type_string(typeof(value)),
			},
		)
	if not (value is Dictionary or value is Array):
		return _error(
			"invalid_state_source",
			"registered state provider must return a Dictionary or Array",
			{
				"provider": str(entry.get("id", "")),
				"method": method_name,
				"expected": "Dictionary or Array",
				"actual": value,
				"actual_type": type_string(typeof(value)),
			},
		)
	return {
		"ok": true,
		"provider": str(entry.get("id", "")),
		"method": method_name,
		"metadata": (entry.get("metadata", {}) as Dictionary).duplicate(true) if entry.get("metadata", {}) is Dictionary else {},
		"state": value,
	}


func call_probe_provider(provider_id: String, request: Dictionary) -> Dictionary:
	var provider := _provider_entry(_probe_providers, provider_id, "probe_provider")
	if not bool(provider.get("ok", false)):
		return provider
	var entry: Dictionary = provider.get("entry", {})
	var owner: Object = entry.get("owner")
	var method_name := str(entry.get("method", ""))
	var request_payload := request.duplicate(true)
	request_payload["provider_id"] = str(entry.get("id", ""))
	var result = owner.call(method_name, request_payload)
	if not _is_json_compatible_project_state_value(result):
		return _error(
			"invalid_probe_result",
			"registered probe provider returned non JSON-compatible data",
			{
				"provider_id": str(entry.get("id", "")),
				"method": method_name,
				"expected": "JSON-compatible Dictionary",
				"actual_type": type_string(typeof(result)),
			},
		)
	if not (result is Dictionary):
		return _error(
			"invalid_probe_result",
			"registered probe provider must return a Dictionary",
			{
				"provider_id": str(entry.get("id", "")),
				"method": method_name,
				"expected": "Dictionary",
				"actual": result,
			},
		)
	var result_dict: Dictionary = result
	result_dict["provider_id"] = str(entry.get("id", ""))
	if entry.get("metadata", {}) is Dictionary and (entry.get("metadata", {}) as Dictionary).has("provider_method"):
		result_dict["provider_method"] = str((entry.get("metadata", {}) as Dictionary).get("provider_method", ""))
	return {"ok": true, "provider_id": str(entry.get("id", "")), "method": method_name, "result": result_dict}


func describe_probe_provider(provider_id: String) -> Dictionary:
	var provider := _provider_entry(_probe_providers, provider_id, "probe_provider")
	if not bool(provider.get("ok", false)):
		return provider
	var entry: Dictionary = provider.get("entry", {})
	return {"ok": true, "data": _provider_public_entry(entry)}


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
	var failures: Array[Dictionary] = []
	var held_actions: Array = _held_actions.keys()
	for action_name in held_actions:
		var result := await execute(
			"input.release_action",
			{
				"action": str(action_name),
			},
		)
		if not bool(result.get("ok", false)):
			failures.append((result.get("error", {}) as Dictionary).duplicate(true))
	return failures


# Zero every non-zero joypad axis injected during this process so a held
# trigger/stick cannot leak into the next warm scenario. Joypad axis state is
# process-global, so this clears the shared tracking set across runtime
# instances. Safe (and a no-op for untouched axes) on the cold one-shot path.
func release_held_axes() -> Array[Dictionary]:
	var axes: Array = _touched_joy_axes.values()
	for entry in axes:
		if not entry is Dictionary:
			continue
		_emit_joy_motion_event(
			int((entry as Dictionary).get("device", 0)),
			int((entry as Dictionary).get("axis_index", -1)),
			0.0,
		)
	_touched_joy_axes.clear()
	if not axes.is_empty():
		await _process_frames(1)
	return []


# Release every joypad button still pressed by injected events (a
# policy.drive_tick hold that never released, or an aborted step). Same
# process-global cleanup contract as release_held_axes.
func release_held_buttons() -> Array[Dictionary]:
	var buttons: Array = _touched_joy_buttons.values()
	for entry in buttons:
		if not entry is Dictionary:
			continue
		_emit_joy_button_event(
			int((entry as Dictionary).get("device", 0)),
			int((entry as Dictionary).get("button_index", -1)),
			false,
		)
	_touched_joy_buttons.clear()
	if not buttons.is_empty():
		await _process_frames(1)
	return []


# A virtual disconnect must release only the events injected for those
# devices. Real controller state and other QA pads stay untouched.
func _release_injected_joypad_devices(devices: Array[int]) -> void:
	var released := false
	for key in _touched_joy_axes.keys():
		var entry: Dictionary = _touched_joy_axes[key]
		if int(entry.get("device", -1)) in devices:
			_emit_joy_motion_event(int(entry["device"]), int(entry["axis_index"]), 0.0)
			released = true
	for key in _touched_joy_buttons.keys():
		var entry: Dictionary = _touched_joy_buttons[key]
		if int(entry.get("device", -1)) in devices:
			_emit_joy_button_event(int(entry["device"]), int(entry["button_index"]), false)
			released = true
	if released:
		await _process_frames(1)


# Lift every screen touch still held down by injected events (a peek/drag that
# never lifted, or an aborted step) at its last tracked position. A normal
# pressed=false release, not a cancel. Same process-global cleanup contract as
# release_held_buttons.
func release_held_touches() -> Array[Dictionary]:
	var indices: Array = _touched_screen_points.keys()
	indices.sort()
	for index in indices:
		var last_position: Vector2 = (_touched_screen_points[index] as Dictionary).get("position", Vector2.ZERO)
		_emit_screen_touch_event(int(index), last_position, false)
	_touched_screen_points.clear()
	if not indices.is_empty():
		await _process_frames(1)
	return []


# Fire a DISCONNECT joy_connection_changed signal for every joypad announced via
# setup.joy_connect and clear the tracking set, so a QA-announced pad cannot leak
# into the next warm scenario sharing this process — a long-lived game listener
# gets a clean disconnect for each fake, and the static tracking set (which drives
# joy_disconnect's real-pad guard) starts empty next scenario. Same process-global
# cleanup contract as release_held_axes/release_held_buttons; a no-op when nothing
# was faked.
func release_faked_joypads() -> Array[Dictionary]:
	var devices: Array = _faked_joypads.keys()
	var device_ids: Array[int] = []
	for device in devices:
		if int(device) not in Input.get_connected_joypads():
			device_ids.append(int(device))
	await _release_injected_joypad_devices(device_ids)
	for device in devices:
		_faked_joypads.erase(device)
		if int(device) in device_ids:
			Input.joy_connection_changed.emit(int(device), false)
	if not device_ids.is_empty():
		await _process_frames(1)
	return []


# Full input reset: release all held actions, zero all touched joypad axes,
# release all pressed joypad buttons, lift all held screen touches, and
# disconnect all QA-faked joypads. Returns any release failures. Exposed as the
# input.reset bridge command and called from scenario finalize so each scenario
# self-cleans its injected input and faked pad connections.
func reset_input() -> Array[Dictionary]:
	var failures: Array[Dictionary] = await release_held_actions()
	failures.append_array(await release_held_axes())
	failures.append_array(await release_held_buttons())
	failures.append_array(await release_held_touches())
	failures.append_array(await release_faked_joypads())
	return failures


func teardown() -> void:
	if _assertions != null and _assertions.has_method("teardown"):
		_assertions.call("teardown")
	_assertions = null
	_scene_root = null
	_root_window = null
	_runtime_errors = []
	_held_actions = {}


func _tap_action(action: String) -> Dictionary:
	if not InputMap.has_action(action):
		return _action_missing_error(action)

	_emit_action_event(action, true)
	await _process_frames(1)
	_emit_action_event(action, false)
	await _process_frames(1)
	return _ok({"action": action})


func _hold_action(action: String, frames: Variant) -> Dictionary:
	if not InputMap.has_action(action):
		return _action_missing_error(action)
	var frame_count = _coerce_frame_count(frames)
	if frame_count == null:
		return _error(
			"invalid_params",
			"input.hold_action requires a non-negative integer frame count",
			{
				"command": "input.hold_action",
				"expected": "non-negative integer frames",
				"actual": frames,
			},
		)

	_emit_action_event(action, true)
	_held_actions[action] = true
	await _process_frames(frame_count)
	return _ok(
		{
			"action": action,
			"frames": frame_count,
			"held": true,
		}
	)


func _release_action(action: String) -> Dictionary:
	if not _held_actions.has(action):
		return _error(
			"action_not_held",
			"input.release_action requires a currently held action",
			{
				"action": action,
				"expected": "held action tracked by the runtime",
				"actual": "not_held",
			},
		)

	_emit_action_event(action, false)
	_held_actions.erase(action)
	await _process_frames(1)
	return _ok(
		{
			"action": action,
			"held": false,
		}
	)


func _click_target(target: String) -> Dictionary:
	var resolution := resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {
			"ok": false,
			"error": (resolution.get("error", {}) as Dictionary).duplicate(true),
		}

	var target_node = resolution.get("node")
	if not target_node is Control:
		return _error(
			"target_not_clickable",
			"Resolved target is not a clickable Control",
			{
				"target": target,
				"resolved": resolution.get("resolved", ""),
				"actual": describe_node(target_node),
			},
		)

	var control := target_node as Control
	var rect := control.get_global_rect()
	if not control.is_visible_in_tree() or rect.size.x <= 0.0 or rect.size.y <= 0.0:
		return _error(
			"target_not_clickable",
			"Resolved target is not a visible Control with positive size",
			{
				"target": target,
				"resolved": resolution.get("resolved", ""),
				"actual": {
					"visible": control.is_visible_in_tree(),
					"global_rect": QaSnapshot.control_to_dict(control).get("global_rect", []),
				},
			},
		)

	var center := rect.position + (rect.size / 2.0)
	await _emit_left_click(center)
	return _ok(
		{
			"target": target,
			"resolved": resolution.get("resolved", ""),
		}
	)


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
	var name := _safe_audio_artifact_name(str(params.get("name", "capture")))
	var bus := str(params.get("bus", "Master"))
	var audio_dir := str(params.get("audio_dir", ""))
	var wav_path := str(params.get("wav_path", params.get("output_path", "")))
	var requested_duration := float(params.get("duration_seconds", params.get("seconds", 0.0)))
	var max_seconds := clampf(float(params.get("max_seconds", AUDIO_CAPTURE_MAX_SECONDS)), 0.001, AUDIO_CAPTURE_MAX_SECONDS)
	var effective_duration := clampf(requested_duration, 0.0, max_seconds)
	var max_bytes := maxi(1, int(params.get("max_bytes", AUDIO_EXPORT_MAX_BYTES)))
	var stop_after_state := _stop_after_state_params(params)
	var stop_sample_every_frames := maxi(1, int(stop_after_state.get("sample_every_frames", 1))) if not stop_after_state.is_empty() else 1
	var warnings: Array = []
	if requested_duration > max_seconds:
		warnings.append(_audio_warning("audio_capture.duration_too_long", "requested live audio capture duration was clamped", {
			"requested_duration_seconds": requested_duration,
			"effective_duration_seconds": effective_duration,
			"max_seconds": max_seconds,
		}))
	var validation := _validate_audio_output("audio.capture", name, audio_dir, wav_path, effective_duration)
	if not bool(validation.get("ok", false)):
		return validation
	var bus_index := AudioServer.get_bus_index(bus)
	if bus_index < 0:
		return _audio_error(
			"audio_capture.bus_not_found",
			"audio.capture could not find the requested AudioServer bus",
			"audio.capture",
			{"bus": bus, "expected": "existing AudioServer bus", "actual": "missing"},
			warnings,
		)
	var record_effect := AudioEffectRecord.new()
	if record_effect == null or not record_effect.has_method("set_recording_active") or not record_effect.has_method("get_recording"):
		return _audio_error(
			"audio_capture.unavailable",
			"Godot safe bus recording APIs are unavailable",
			"audio.capture",
			{"bus": bus, "expected": "AudioEffectRecord recording API", "actual": "unavailable"},
			warnings,
		)
	var dir_error := DirAccess.make_dir_recursive_absolute(audio_dir)
	if dir_error != OK:
		return _audio_error(
			"audio_capture.failed",
			"audio.capture could not create the controlled audio artifact directory",
			"audio.capture",
			{"path": audio_dir, "expected": "writable controlled audio_dir", "actual": dir_error},
			warnings,
		)
	var effect_index := AudioServer.get_bus_effect_count(bus_index)
	AudioServer.add_bus_effect(bus_index, record_effect, effect_index)
	record_effect.call("set_recording_active", true)
	var started_msec := Time.get_ticks_msec()
	var stop_condition_matched := false
	var stop_condition_sample_count := 0
	var stop_condition_last_error: Dictionary = {}
	while float(Time.get_ticks_msec() - started_msec) / 1000.0 < effective_duration:
		await _process_frames(stop_sample_every_frames)
		if not stop_after_state.is_empty():
			stop_condition_sample_count += 1
			var stop_result: Dictionary = await _assertions.execute("assert.state_matches", stop_after_state)
			if bool(stop_result.get("ok", false)):
				stop_condition_matched = true
				break
			var stop_error: Dictionary = stop_result.get("error", {}) if stop_result.get("error", {}) is Dictionary else {}
			stop_condition_last_error = stop_error.duplicate(true)
			if str(stop_error.get("type", "")) != "state_matches_mismatch":
				record_effect.call("set_recording_active", false)
				AudioServer.remove_bus_effect(bus_index, effect_index)
				return {"ok": false, "error": stop_error}
	record_effect.call("set_recording_active", false)
	var recording = record_effect.call("get_recording")
	AudioServer.remove_bus_effect(bus_index, effect_index)
	if recording == null or not recording is AudioStreamWAV:
		return _audio_error(
			"audio_capture.failed",
			"audio.capture did not produce WAV recording data",
			"audio.capture",
			{"bus": bus, "expected": "AudioStreamWAV recording", "actual": _json_compatible_value(recording)},
			warnings,
		)
	var save_error: int = (recording as AudioStreamWAV).save_to_wav(wav_path)
	if save_error != OK or not FileAccess.file_exists(wav_path):
		return _audio_error(
			"audio_capture.failed",
			"audio.capture could not write the WAV artifact",
			"audio.capture",
			{"path": wav_path, "expected": "writable WAV path under audio_dir", "actual": save_error},
			warnings,
		)
	var file := FileAccess.open(wav_path, FileAccess.READ)
	var byte_count := file.get_length() if file != null else 0
	if byte_count <= 0:
		return _audio_error(
			"audio_capture.failed",
			"audio.capture captured no audio frames; the runtime may be running without an active audio output device",
			"audio.capture",
			{
				"path": wav_path,
				"bus": bus,
				"expected": "non-empty WAV recording",
				"actual": byte_count,
				"remediation": "Confirm the live session has an audio output device (or dummy-driver mixing) and that the selected bus produces sound during the capture window.",
			},
			warnings,
		)
	if byte_count > max_bytes:
		return _audio_error(
			"audio_capture.failed",
			"audio.capture wrote an oversized WAV artifact",
			"audio.capture",
			{"path": wav_path, "expected": "1..%d bytes" % max_bytes, "actual": byte_count},
			warnings,
		)
	var artifacts := _audio_artifacts(name, wav_path, audio_dir, {"source": "audio.capture", "bus": bus, "bytes": byte_count})
	var payload := {
		"name": name,
		"bus": bus,
		"duration_seconds": requested_duration,
		"effective_duration_seconds": effective_duration,
		"capabilities": {"bus_capture": true, "host_microphone_capture": false},
		"artifacts": artifacts.get(name, {}),
		"audio_artifacts": artifacts,
		"warnings": warnings,
		"wav_path": wav_path,
	}
	if not stop_after_state.is_empty():
		payload["stop_after_state"] = {
			"requested": stop_after_state.duplicate(true),
			"matched": stop_condition_matched,
			"sample_count": stop_condition_sample_count,
			"sample_every_frames": stop_sample_every_frames,
			"last_error": stop_condition_last_error,
		}
		payload["stop_condition_matched"] = stop_condition_matched
		payload["stop_condition_timed_out"] = not stop_condition_matched
	return _ok(payload)


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
	var name := _safe_audio_artifact_name(str(params.get("name", "export")))
	var target := str(params.get("target", ""))
	var method_name := str(params.get("method", ""))
	var audio_dir := str(params.get("audio_dir", ""))
	var wav_path := str(params.get("wav_path", params.get("output_path", "")))
	var max_bytes := maxi(1, int(params.get("max_bytes", AUDIO_EXPORT_MAX_BYTES)))
	var validation := _validate_audio_output("audio.export", name, audio_dir, wav_path, 1.0)
	if not bool(validation.get("ok", false)):
		return validation
	if target == "":
		return _audio_error("invalid_params", "audio.export requires a target", "audio.export", {"expected": "non-empty target", "actual": target})
	if not method_name.begins_with("qa_"):
		return _audio_error(
			"audio_export.rejected",
			"audio.export only allows project-owned qa_ methods",
			"audio.export",
			{"target": target, "method": method_name, "expected": "method name beginning with qa_", "actual": method_name},
		)
	var resolution := resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var node := resolution.get("node") as Node
	if node == null or not node.has_method(method_name):
		return _audio_error(
			"audio_export.rejected",
			"audio.export could not find the requested qa_ method",
			"audio.export",
			{"target": target, "method": method_name, "expected": "method on resolved node", "actual": node.get_class() if node != null else "missing"},
		)
	var dir_error := DirAccess.make_dir_recursive_absolute(audio_dir)
	if dir_error != OK:
		return _audio_error("audio_export.failed", "audio.export could not create the controlled audio artifact directory", "audio.export", {"path": audio_dir, "expected": "writable controlled audio_dir", "actual": dir_error})
	var args_value = params.get("args", [])
	var args: Array = args_value if args_value is Array else []
	var call_args := args.duplicate(true)
	call_args.append({"audio_dir": audio_dir, "name": name, "wav_path": wav_path, "max_bytes": max_bytes})
	var result = node.callv(method_name, call_args)
	await _process_frames(1)
	if result is PackedByteArray:
		return _accept_audio_export_bytes(name, wav_path, audio_dir, result, target, method_name, max_bytes)
	if result is String:
		return _accept_audio_export_bytes(name, wav_path, audio_dir, Marshalls.base64_to_raw(str(result)), target, method_name, max_bytes)
	if result is Dictionary:
		return _accept_audio_export_dictionary(name, audio_dir, result, target, method_name, max_bytes)
	return _audio_error(
		"audio_export.invalid_return",
		"audio.export method returned an unsupported value",
		"audio.export",
		{"target": target, "method": method_name, "expected": "base64 WAV bytes, PackedByteArray, or dictionary describing a WAV under audio_dir", "actual": _json_compatible_value(result)},
	)


func _accept_audio_export_bytes(name: String, wav_path: String, audio_dir: String, bytes: PackedByteArray, target: String, method_name: String, max_bytes: int) -> Dictionary:
	if bytes.size() <= 0 or bytes.size() > max_bytes:
		return _audio_error("audio_export.oversized", "audio.export returned an empty or oversized WAV payload", "audio.export", {"target": target, "method": method_name, "expected": "1..%d bytes" % max_bytes, "actual": bytes.size()})
	if not _looks_like_wav(bytes):
		return _audio_error("audio_export.invalid_return", "audio.export returned bytes that are not RIFF/WAVE data", "audio.export", {"target": target, "method": method_name, "expected": "RIFF/WAVE bytes", "actual": "invalid_header"})
	var file := FileAccess.open(wav_path, FileAccess.WRITE)
	if file == null:
		return _audio_error("audio_export.failed", "audio.export could not write the WAV artifact", "audio.export", {"target": target, "method": method_name, "expected": wav_path, "actual": "open_failed"})
	file.store_buffer(bytes)
	var artifacts := _audio_artifacts(name, wav_path, audio_dir, {"source": "audio.export", "target": target, "method": method_name, "bytes": bytes.size()})
	return _ok({"name": name, "target": target, "method": method_name, "artifacts": artifacts.get(name, {}), "audio_artifacts": artifacts, "warnings": [], "wav_path": wav_path})


func _accept_audio_export_dictionary(name: String, audio_dir: String, result: Dictionary, target: String, method_name: String, max_bytes: int) -> Dictionary:
	if not _is_json_compatible_audio_value(result):
		return _audio_error("audio_export.invalid_metadata", "audio.export returned non-JSON-compatible metadata", "audio.export", {"target": target, "method": method_name, "expected": "JSON-compatible dictionary", "actual": _json_compatible_value(result)})
	var wav_path := str(result.get("wav", result.get("path", "")))
	if not _is_path_under_audio_dir(audio_dir, wav_path):
		return _audio_error("audio_export.path_rejected", "audio.export returned a WAV path outside the controlled audio directory", "audio.export", {"target": target, "method": method_name, "expected": audio_dir, "actual": wav_path})
	var file := FileAccess.open(wav_path, FileAccess.READ)
	var byte_count := file.get_length() if file != null else 0
	if byte_count <= 0 or byte_count > max_bytes:
		return _audio_error("audio_export.oversized", "audio.export wrote an empty or oversized WAV artifact", "audio.export", {"target": target, "method": method_name, "expected": "1..%d bytes" % max_bytes, "actual": byte_count})
	if file == null or not _looks_like_wav(file.get_buffer(mini(byte_count, 12))):
		return _audio_error("audio_export.invalid_return", "audio.export wrote a file that is not RIFF/WAVE data", "audio.export", {"target": target, "method": method_name, "expected": "RIFF/WAVE file", "actual": wav_path})
	var metadata := result.duplicate(true)
	metadata.erase("wav")
	metadata.erase("path")
	metadata["source"] = "audio.export"
	metadata["target"] = target
	metadata["method"] = method_name
	metadata["bytes"] = byte_count
	var artifacts := _audio_artifacts(name, wav_path, audio_dir, metadata)
	return _ok({"name": name, "target": target, "method": method_name, "artifacts": artifacts.get(name, {}), "audio_artifacts": artifacts, "warnings": [], "wav_path": wav_path})


## Offline-bake: the runtime owns the loop length (from seconds * mix_rate) and the WAV
## container; the project supplies only a qa_ voice-fill method returning mono samples in
## [-1, 1]. This generalizes per-project tools like coop-2's tools/bake_audio.gd and works
## headless because nothing is captured from an output device — it is synthesized to a buffer.
func _audio_render(params: Dictionary) -> Dictionary:
	var name := _safe_audio_artifact_name(str(params.get("name", "render")))
	var target := str(params.get("target", ""))
	var method_name := str(params.get("method", ""))
	var audio_dir := str(params.get("audio_dir", ""))
	var wav_path := str(params.get("wav_path", params.get("output_path", "")))
	var max_bytes := maxi(1, int(params.get("max_bytes", AUDIO_EXPORT_MAX_BYTES)))
	var mix_rate := maxi(1, int(params.get("mix_rate", 44100)))
	var duration_seconds := float(params.get("effective_duration_seconds", params.get("duration_seconds", 0.0)))
	var validation := _validate_audio_output("audio.render", name, audio_dir, wav_path, duration_seconds)
	if not bool(validation.get("ok", false)):
		return validation
	if target == "":
		return _audio_error("invalid_params", "audio.render requires a target", "audio.render", {"expected": "non-empty target", "actual": target})
	if not method_name.begins_with("qa_"):
		return _audio_error(
			"audio_render.rejected",
			"audio.render only allows project-owned qa_ methods",
			"audio.render",
			{"target": target, "method": method_name, "expected": "method name beginning with qa_", "actual": method_name},
		)
	var frame_count := int(ceil(duration_seconds * float(mix_rate)))
	if frame_count <= 0 or frame_count * 2 > max_bytes:
		return _audio_error(
			"audio_render.oversized",
			"audio.render requested duration is empty or would exceed the byte budget",
			"audio.render",
			{"target": target, "method": method_name, "expected": "1..%d frames" % (max_bytes / 2), "actual": frame_count},
		)
	var resolution := resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var node := resolution.get("node") as Node
	if node == null or not node.has_method(method_name):
		return _audio_error(
			"audio_render.rejected",
			"audio.render could not find the requested qa_ method",
			"audio.render",
			{"target": target, "method": method_name, "expected": "method on resolved node", "actual": node.get_class() if node != null else "missing"},
		)
	var dir_error := DirAccess.make_dir_recursive_absolute(audio_dir)
	if dir_error != OK:
		return _audio_error("audio_render.failed", "audio.render could not create the controlled audio artifact directory", "audio.render", {"path": audio_dir, "expected": "writable controlled audio_dir", "actual": dir_error})
	var args_value = params.get("args", [])
	var args: Array = args_value if args_value is Array else []
	var call_args: Array = [frame_count, mix_rate]
	call_args.append_array(args)
	var result = node.callv(method_name, call_args)
	await _process_frames(1)
	var frames := _audio_render_frames(result)
	if frames == null:
		return _audio_error(
			"audio_render.invalid_return",
			"audio.render method returned an unsupported value",
			"audio.render",
			{"target": target, "method": method_name, "expected": "PackedFloat32Array, PackedVector2Array, or Array of mono samples in [-1, 1]", "actual": _json_compatible_value(result)},
		)
	if (frames as PackedFloat32Array).is_empty():
		return _audio_error("audio_render.failed", "audio.render produced no audio frames", "audio.render", {"target": target, "method": method_name, "expected": "non-empty sample buffer", "actual": 0})
	var wav := _audio_render_wav(frames as PackedFloat32Array, mix_rate)
	if wav.data.size() > max_bytes:
		return _audio_error("audio_render.oversized", "audio.render produced an oversized WAV payload", "audio.render", {"target": target, "method": method_name, "expected": "1..%d bytes" % max_bytes, "actual": wav.data.size()})
	if wav.save_to_wav(wav_path) != OK:
		return _audio_error("audio_render.failed", "audio.render could not write the WAV artifact", "audio.render", {"target": target, "method": method_name, "expected": wav_path, "actual": "save_failed"})
	var file := FileAccess.open(wav_path, FileAccess.READ)
	var byte_count := file.get_length() if file != null else 0
	if file == null or byte_count <= 0 or byte_count > max_bytes:
		return _audio_error("audio_render.oversized", "audio.render wrote an empty or oversized WAV artifact", "audio.render", {"target": target, "method": method_name, "expected": "1..%d bytes" % max_bytes, "actual": byte_count})
	if not _looks_like_wav(file.get_buffer(mini(byte_count, 12))):
		return _audio_error("audio_render.invalid_return", "audio.render wrote a file that is not RIFF/WAVE data", "audio.render", {"target": target, "method": method_name, "expected": "RIFF/WAVE file", "actual": wav_path})
	var metadata := {"source": "audio.render", "target": target, "method": method_name, "frames": (frames as PackedFloat32Array).size(), "mix_rate": mix_rate, "bytes": byte_count}
	var artifacts := _audio_artifacts(name, wav_path, audio_dir, metadata)
	return _ok({"name": name, "target": target, "method": method_name, "artifacts": artifacts.get(name, {}), "audio_artifacts": artifacts, "warnings": [], "wav_path": wav_path})


## Coerce a qa_ render return value into a mono float buffer, or null if unsupported.
func _audio_render_frames(result: Variant) -> Variant:
	if result is PackedFloat32Array:
		return result
	if result is PackedVector2Array:
		var stereo := result as PackedVector2Array
		var downmixed := PackedFloat32Array()
		downmixed.resize(stereo.size())
		for index in stereo.size():
			downmixed[index] = (stereo[index].x + stereo[index].y) * 0.5
		return downmixed
	if result is Array:
		var coerced := PackedFloat32Array()
		coerced.resize((result as Array).size())
		for index in (result as Array).size():
			var value = (result as Array)[index]
			if not (value is int or value is float):
				return null
			coerced[index] = float(value)
		return coerced
	return null


## Pack mono float samples in [-1, 1] into a 16-bit PCM AudioStreamWAV (same container the
## live AudioStreamGenerator feeds), matching coop-2's tools/bake_audio.gd `_bake_music`.
func _audio_render_wav(frames: PackedFloat32Array, mix_rate: int) -> AudioStreamWAV:
	var pcm := PackedByteArray()
	pcm.resize(frames.size() * 2)
	for index in frames.size():
		var sample := int(round(clampf(frames[index], -1.0, 1.0) * 32767.0))
		pcm.encode_s16(index * 2, sample)
	var wav := AudioStreamWAV.new()
	wav.format = AudioStreamWAV.FORMAT_16_BITS
	wav.mix_rate = mix_rate
	wav.stereo = false
	wav.data = pcm
	return wav


func _validate_audio_output(command: String, name: String, audio_dir: String, wav_path: String, duration_seconds: float) -> Dictionary:
	if name == "" or audio_dir == "" or wav_path == "" or duration_seconds <= 0.0 or not is_finite(duration_seconds):
		return _audio_error(
			"invalid_params",
			"%s requires a non-empty name, controlled audio_dir, WAV path, and positive bounded duration" % command,
			command,
			{"expected": "valid name, audio_dir, wav_path, duration_seconds > 0", "actual": {"name": name, "audio_dir": audio_dir, "wav_path": wav_path, "duration_seconds": duration_seconds}},
		)
	if not _is_path_under_audio_dir(audio_dir, wav_path):
		return _audio_error(
			"invalid_params",
			"%s WAV path must stay under the controlled audio_dir" % command,
			command,
			{"expected": audio_dir, "actual": wav_path},
		)
	return {"ok": true}


func _audio_artifacts(name: String, wav_path: String, audio_dir: String, metadata: Dictionary) -> Dictionary:
	return {
		name: {
			"wav": wav_path,
			"name": name,
			"metadata": metadata.duplicate(true),
		},
		"root": audio_dir,
		"wav": wav_path,
	}


func _audio_warning(warning_id: String, message: String, data := {}) -> Dictionary:
	var warning := {"id": warning_id, "message": message}
	if data is Dictionary and not data.is_empty():
		warning["data"] = data.duplicate(true)
	return warning


func _audio_error(error_type: String, message: String, command: String, data := {}, warnings := []) -> Dictionary:
	var payload: Dictionary = data.duplicate(true) if data is Dictionary else {}
	payload["command"] = command
	if warnings is Array and not warnings.is_empty():
		payload["warnings"] = warnings.duplicate(true)
	return _error(error_type, message, payload)


func _safe_audio_artifact_name(name: String) -> String:
	var safe := ""
	for index in range(name.length()):
		var character := name.substr(index, 1)
		if character.is_valid_identifier() or character.is_valid_int() or character in ["-", "_"]:
			safe += character
		else:
			safe += "_"
	return safe.strip_edges().trim_prefix(".")


func _is_path_under_audio_dir(audio_dir: String, path: String) -> bool:
	var simplified_audio_dir := audio_dir.simplify_path()
	var simplified_path := path.simplify_path()
	return simplified_audio_dir != "" and simplified_path.ends_with(".wav") and (simplified_path == simplified_audio_dir or simplified_path.begins_with("%s/" % simplified_audio_dir))


func _looks_like_wav(bytes: PackedByteArray) -> bool:
	return bytes.size() >= 12 and bytes.slice(0, 4).get_string_from_ascii() == "RIFF" and bytes.slice(8, 12).get_string_from_ascii() == "WAVE"


func _is_json_compatible_audio_value(value: Variant) -> bool:
	if value == null or value is bool or value is int or value is float or value is String:
		return true
	if value is Array:
		for item in value:
			if not _is_json_compatible_audio_value(item):
				return false
		return true
	if value is Dictionary:
		for key in value.keys():
			if not (key is String or key is StringName):
				return false
			if not _is_json_compatible_audio_value(value.get(key)):
				return false
		return true
	return false


func _trace_record(params: Dictionary) -> Dictionary:
	var raw_targets: Array = params.get("targets", []) if params.get("targets", []) is Array else []
	if raw_targets.is_empty():
		return _error(
			"invalid_params",
			"trace.record requires at least one target",
			{
				"expected": "non-empty targets array",
				"actual": params.get("targets", null),
			},
		)
	var resolved_targets: Array = []
	for raw_target in raw_targets:
		var target := str(raw_target).strip_edges()
		if target == "":
			return _error(
				"invalid_params",
				"trace.record target must be non-empty",
				{
					"expected": "non-empty target locator",
					"actual": raw_target,
				},
			)
		var resolution := resolve_target(target)
		if not bool(resolution.get("ok", false)):
			return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
		resolved_targets.append({
			"target": target,
			"resolution": resolution,
			"node": resolution.get("node") as Node,
		})

	var project_state_preflight := _trace_project_state_preflight(params)
	if not bool(project_state_preflight.get("ok", false)):
		return project_state_preflight

	var frames := clampi(int(params.get("frames", 180)), 1, 3600)
	var sample_every_frames := clampi(int(params.get("sample_every_frames", 1)), 1, 240)
	var include_physics := bool(params.get("include_physics", false))
	var radius := clampf(float(params.get("radius", 256.0)), 1.0, 4096.0)
	var trace_id := "trace-%d-%d" % [Time.get_unix_time_from_system(), Time.get_ticks_msec()]
	var stop_after_state := _stop_after_state_params(params)
	var samples: Array = []
	var events: Array = []
	var elapsed_frames := 0
	var sample_index := 0
	var previous_sample: Dictionary = {}
	var stop_condition_matched := false
	var stop_condition_sample_count := 0
	var stop_condition_frame := -1
	var stop_condition_last_error: Dictionary = {}

	while elapsed_frames <= frames:
		var sample := _trace_capture_sample(
			resolved_targets,
			params,
			sample_index,
			elapsed_frames,
			include_physics,
			radius,
		)
		samples.append(sample)
		events.append_array(_trace_derive_events(previous_sample, sample))
		previous_sample = sample
		if not stop_after_state.is_empty():
			stop_condition_sample_count += 1
			var stop_result: Dictionary = await _assertions.execute("assert.state_matches", stop_after_state)
			if bool(stop_result.get("ok", false)):
				stop_condition_matched = true
				stop_condition_frame = elapsed_frames
				break
			var stop_error: Dictionary = stop_result.get("error", {}) if stop_result.get("error", {}) is Dictionary else {}
			stop_condition_last_error = stop_error.duplicate(true)
			if str(stop_error.get("type", "")) != "state_matches_mismatch":
				return {"ok": false, "error": stop_error}
		if elapsed_frames == frames:
			break
		var wait_frames := mini(sample_every_frames, frames - elapsed_frames)
		await _process_frames(wait_frames)
		elapsed_frames += wait_frames
		sample_index += 1

	var payload := {
		"trace_id": trace_id,
		"session": {
			"scene": str(_scene_root.scene_file_path) if _scene_root != null else "",
			"started_frame": samples[0].get("frame", 0) if not samples.is_empty() else Engine.get_process_frames(),
			"ended_frame": samples[samples.size() - 1].get("frame", 0) if not samples.is_empty() else Engine.get_process_frames(),
			"started_time_msec": samples[0].get("time_msec", 0) if not samples.is_empty() else Time.get_ticks_msec(),
			"ended_time_msec": samples[samples.size() - 1].get("time_msec", 0) if not samples.is_empty() else Time.get_ticks_msec(),
		},
		"config": {
			"frames": frames,
			"sample_every_frames": sample_every_frames,
			"include_physics": include_physics,
			"physics_radius": radius,
			"include_project_state": bool(params.get("include_project_state", false)),
			"state_provider": str(params.get("state_provider", "scene-root")),
			"state_methods": params.get("state_methods", []) if params.get("state_methods", []) is Array else [],
		},
		"targets": _trace_target_summaries(resolved_targets),
		"samples": samples,
		"events": events,
		"summary": {
			"target_count": resolved_targets.size(),
			"sample_count": samples.size(),
			"event_count": events.size(),
			"event_counts": _trace_event_counts(events),
			"physics_enabled": include_physics,
			"project_state_enabled": bool(params.get("include_project_state", false)),
		},
	}
	if not stop_after_state.is_empty():
		payload["stop_after_state"] = {
			"requested": stop_after_state.duplicate(true),
			"matched": stop_condition_matched,
			"sample_count": stop_condition_sample_count,
			"stopped_at_frame": stop_condition_frame,
			"last_error": stop_condition_last_error,
		}
		payload["stop_condition_matched"] = stop_condition_matched
		payload["stop_condition_timed_out"] = not stop_condition_matched
	return _ok(payload)


func _event_trace(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", "")).strip_edges()
	if target == "":
		return _error(
			"invalid_params",
			"event.trace requires a non-empty target",
			{
				"command": "event.trace",
				"field": "target",
				"expected": "non-empty target locator",
				"actual": params.get("target", null),
			},
		)
	var provider_method := str(params.get("provider", "")).strip_edges()
	if provider_method == "":
		return _error(
			"event_trace.provider_missing",
			"event.trace requires an explicit approved event provider method",
			{
				"command": "event.trace",
				"field": "provider",
				"expected": "get_qa_event_stream_state, get_event_stream_qa_state, get_qa_events, or qa_event_stream",
				"actual": params.get("provider", null),
			},
		)
	if not _is_allowed_event_provider_method(provider_method):
		return _error(
			"event_trace.provider_invalid",
			"event.trace provider is not an approved read-only event provider method",
			{
				"command": "event.trace",
				"field": "provider",
				"expected": "get_qa_event_stream_state, get_event_stream_qa_state, get_qa_events, or qa_event_stream",
				"actual": provider_method,
			},
		)
	var resolution := resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var provider := resolution.get("node") as Node
	if provider == null or not provider.has_method(provider_method):
		return _error(
			"event_trace.provider_missing",
			"event.trace target does not expose the requested event provider",
			{
				"command": "event.trace",
				"target": target,
				"provider": provider_method,
				"expected": "provider method present on resolved target",
				"actual": "missing",
			},
		)
	if not _method_has_zero_required_arguments(provider, provider_method):
		return _error(
			"event_trace.provider_invalid",
			"event.trace providers must take zero required arguments",
			{
				"command": "event.trace",
				"target": target,
				"provider": provider_method,
				"expected": "zero required arguments",
				"actual": "requires arguments",
			},
		)

	var frames := clampi(int(params.get("frames", 180)), 1, 3600)
	var sample_every_frames := clampi(int(params.get("sample_every_frames", 1)), 1, 240)
	var max_events_per_sample := clampi(int(params.get("max_events_per_sample", 512)), 1, 4096)
	var trace_id := "event-trace-%d-%d" % [Time.get_unix_time_from_system(), Time.get_ticks_msec()]
	var samples: Array = []
	var events: Array = []
	var warnings: Array = []
	var provider_error_count := 0
	var elapsed_frames := 0
	var sample_index := 0

	while elapsed_frames <= frames:
		var sample_frame := Engine.get_process_frames()
		var sample_time_msec := Time.get_ticks_msec()
		var provider_result := _call_event_provider(provider, provider_method)
		if not bool(provider_result.get("ok", false)):
			provider_error_count += 1
			warnings.append(_event_trace_warning(
				_event_provider_warning_id(str(provider_result.get("status", ""))),
				"event.trace provider sample failed",
				{
					"target": target,
					"provider": provider_method,
					"sample_index": sample_index,
					"elapsed_frames": elapsed_frames,
					"frame": sample_frame,
					"status": provider_result.get("status", "provider_error"),
					"provider_result": provider_result,
				},
			))
			samples.append({
				"index": sample_index,
				"elapsed_frames": elapsed_frames,
				"frame": sample_frame,
				"time_msec": sample_time_msec,
				"event_count": 0,
				"events": [],
				"provider_status": str(provider_result.get("status", "provider_error")),
			})
		else:
			var normalized := _normalize_event_stream_value(
				provider_result.get("value", null),
				sample_index,
				elapsed_frames,
				sample_frame,
				sample_time_msec,
				max_events_per_sample,
			)
			var sample_events: Array = normalized.get("events", []) if normalized.get("events", []) is Array else []
			events.append_array(sample_events)
			for warning in (normalized.get("warnings", []) if normalized.get("warnings", []) is Array else []):
				warnings.append(warning)
			samples.append({
				"index": sample_index,
				"elapsed_frames": elapsed_frames,
				"frame": sample_frame,
				"time_msec": sample_time_msec,
				"event_count": int(normalized.get("event_count", sample_events.size())),
				"returned_event_count": sample_events.size(),
				"events": sample_events,
				"provider_status": "ok",
				"truncated": bool(normalized.get("truncated", false)),
			})

		if elapsed_frames == frames:
			break
		var wait_frames := mini(sample_every_frames, frames - elapsed_frames)
		await _process_frames(wait_frames)
		elapsed_frames += wait_frames
		sample_index += 1

	if events.is_empty():
		warnings.append(_event_trace_warning(
			"event_trace.no_events_recorded",
			"event.trace recorded no events",
			{
				"target": target,
				"provider": provider_method,
				"frames": frames,
				"sample_every_frames": sample_every_frames,
			},
		))

	return _ok({
		"trace_id": trace_id,
		"session": {
			"scene": str(_scene_root.scene_file_path) if _scene_root != null else "",
			"started_frame": samples[0].get("frame", 0) if not samples.is_empty() else Engine.get_process_frames(),
			"ended_frame": samples[samples.size() - 1].get("frame", 0) if not samples.is_empty() else Engine.get_process_frames(),
			"started_time_msec": samples[0].get("time_msec", 0) if not samples.is_empty() else Time.get_ticks_msec(),
			"ended_time_msec": samples[samples.size() - 1].get("time_msec", 0) if not samples.is_empty() else Time.get_ticks_msec(),
		},
		"config": {
			"frames": frames,
			"sample_every_frames": sample_every_frames,
			"max_events_per_sample": max_events_per_sample,
		},
		"target": {
			"requested": target,
			"resolved": str(resolution.get("resolved", "path:%s" % str(provider.get_path()))),
			"path": str(provider.get_path()),
			"type": provider.get_class(),
		},
		"provider": {
			"method": provider_method,
			"allowed": true,
			"zero_required_arguments": _method_has_zero_required_arguments(provider, provider_method),
		},
		"viewport": _event_trace_viewport_metadata(),
		"samples": samples,
		"events": events,
		"warnings": warnings,
		"provider_error_count": provider_error_count,
		"summary": {
			"sample_count": samples.size(),
			"total_event_count": events.size(),
			"provider_error_count": provider_error_count,
			"warning_count": warnings.size(),
		},
	})


# Sample one read-only qa_ state source every N frames into a bounded row
# buffer for telemetry evidence (pacing/difficulty curves). Rows are capped at
# MAX_TELEMETRY_ROWS so a long window can never grow unbounded in memory; the
# recording reports `truncated: true` when the cap stops collection early.
func _telemetry_record(params: Dictionary) -> Dictionary:
	var provider_id := str(params.get("provider", "")).strip_edges()
	var target := str(params.get("target", "")).strip_edges()
	var method := str(params.get("method", "")).strip_edges()
	if provider_id == "" and (target == "" or method == ""):
		return _error(
			"invalid_params",
			"telemetry.record requires a provider id or a target with a method",
			{
				"command": "telemetry.record",
				"expected": "provider, or target + method",
				"actual": {"provider": params.get("provider", null), "target": params.get("target", null), "method": params.get("method", null)},
			},
		)
	var source := {}
	var provider_node: Node = null
	if provider_id != "":
		var provider_probe := read_state_provider(provider_id)
		if not bool(provider_probe.get("ok", false)):
			return provider_probe
		source = {"provider": provider_id, "method": str(provider_probe.get("method", ""))}
	else:
		if not _is_allowed_telemetry_method(method):
			return _error(
				"telemetry.method_invalid",
				"telemetry.record method must be a read-only qa_ state method",
				{
					"command": "telemetry.record",
					"field": "method",
					"expected": "qa_* (non-mutating), get_qa_*_state, or get_*_qa_state",
					"actual": method,
				},
			)
		var resolution := resolve_target(target)
		if not bool(resolution.get("ok", false)):
			return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
		provider_node = resolution.get("node") as Node
		if provider_node == null or not provider_node.has_method(method):
			return _error(
				"telemetry.provider_missing",
				"telemetry.record target does not expose the requested method",
				{
					"command": "telemetry.record",
					"target": target,
					"method": method,
					"expected": "method present on resolved target",
					"actual": "missing",
				},
			)
		if not _method_has_zero_required_arguments(provider_node, method):
			return _error(
				"telemetry.method_invalid",
				"telemetry.record methods must take zero required arguments",
				{
					"command": "telemetry.record",
					"target": target,
					"method": method,
					"expected": "zero required arguments",
					"actual": "requires arguments",
				},
			)
		source = {"target": target, "method": method}

	var frames := clampi(int(params.get("frames", 600)), 1, 3600)
	var every := clampi(int(params.get("every", 1)), 1, 240)
	var telemetry_id := "telemetry-%d-%d" % [Time.get_unix_time_from_system(), Time.get_ticks_msec()]
	var rows: Array = []
	var warnings: Array = []
	var provider_error_count := 0
	var truncated := false
	var elapsed_frames := 0

	while elapsed_frames <= frames:
		if rows.size() >= MAX_TELEMETRY_ROWS:
			truncated = true
			break
		var row := {
			"frame": Engine.get_process_frames(),
			"t_ms": Time.get_ticks_msec(),
			"value": null,
		}
		if provider_id != "":
			var provider_result := read_state_provider(provider_id)
			if bool(provider_result.get("ok", false)):
				row["value"] = provider_result.get("state", null)
			else:
				provider_error_count += 1
				row["error"] = str(((provider_result.get("error", {}) as Dictionary)).get("type", "provider_error"))
		else:
			var sample := _call_telemetry_method(provider_node, method)
			if bool(sample.get("ok", false)):
				row["value"] = sample.get("value", null)
			else:
				provider_error_count += 1
				row["error"] = str(sample.get("status", "provider_error"))
		if row.has("error") and warnings.size() < 32:
			warnings.append({
				"id": "telemetry.sample_failed",
				"message": "telemetry.record sample failed",
				"data": {"elapsed_frames": elapsed_frames, "error": row.get("error", "")},
			})
		rows.append(row)
		if elapsed_frames == frames:
			break
		var wait_frames := mini(every, frames - elapsed_frames)
		await _process_frames(wait_frames)
		elapsed_frames += wait_frames

	var config := {
		"frames": frames,
		"every": every,
		"max_rows": MAX_TELEMETRY_ROWS,
	}
	for key in source.keys():
		config[key] = source[key]
	return _ok({
		"telemetry_id": telemetry_id,
		"session": {
			"scene": str(_scene_root.scene_file_path) if _scene_root != null else "",
			"started_frame": rows[0].get("frame", 0) if not rows.is_empty() else Engine.get_process_frames(),
			"ended_frame": rows[rows.size() - 1].get("frame", 0) if not rows.is_empty() else Engine.get_process_frames(),
			"started_time_msec": rows[0].get("t_ms", 0) if not rows.is_empty() else Time.get_ticks_msec(),
			"ended_time_msec": rows[rows.size() - 1].get("t_ms", 0) if not rows.is_empty() else Time.get_ticks_msec(),
		},
		"config": config,
		"rows": rows,
		"row_count": rows.size(),
		"truncated": truncated,
		"provider_error_count": provider_error_count,
		"warnings": warnings,
	})


# Telemetry samples must stay read-only: accept the read-only QA state naming
# contract plus plain qa_-prefixed provider methods, while rejecting the
# mutating qa_prepare_/qa_apply_/qa_mutate_/qa_set_ fixture prefixes. Mirrors
# is_allowed_telemetry_method on the CLI side.
func _is_allowed_telemetry_method(method: String) -> bool:
	if method == "" \
			or method.begins_with("qa_prepare_") \
			or method.begins_with("qa_apply_") \
			or method.begins_with("qa_mutate_") \
			or method.begins_with("qa_set_"):
		return false
	for character in method:
		var codepoint := character.unicode_at(0)
		if not ((codepoint >= 48 and codepoint <= 57) or (codepoint >= 65 and codepoint <= 90) or (codepoint >= 97 and codepoint <= 122) or character == "_"):
			return false
	return method.begins_with("qa_") \
		or (method.begins_with("get_qa_") and method.ends_with("_state")) \
		or (method.begins_with("get_") and method.ends_with("_qa_state"))


func _call_telemetry_method(provider: Node, method: String) -> Dictionary:
	if provider == null or not is_instance_valid(provider):
		return {"ok": false, "status": "provider_freed"}
	if not provider.has_method(method):
		return {"ok": false, "status": "missing_method"}
	var runtime_error_count_before := _runtime_errors.size()
	var value = provider.call(method)
	if _runtime_errors.size() > runtime_error_count_before:
		return {"ok": false, "status": "provider_runtime_error"}
	if not _is_json_compatible_project_state_value(value):
		return {"ok": false, "status": "non_json_compatible_value"}
	return {"ok": true, "value": value}


func _is_allowed_event_provider_method(method: String) -> bool:
	return method in [
		"get_qa_event_stream_state",
		"get_event_stream_qa_state",
		"get_qa_events",
		"qa_event_stream",
	]


func _call_event_provider(provider: Node, method: String) -> Dictionary:
	if not _is_allowed_event_provider_method(method):
		return {"method": method, "ok": false, "status": "rejected_method"}
	if provider == null or not provider.has_method(method):
		return {"method": method, "ok": false, "status": "missing_method"}
	if not _method_has_zero_required_arguments(provider, method):
		return {"method": method, "ok": false, "status": "requires_arguments"}
	var runtime_error_count_before := _runtime_errors.size()
	var value = provider.call(method)
	if _runtime_errors.size() > runtime_error_count_before:
		return {
			"method": method,
			"ok": false,
			"status": "runtime_error",
			"runtime_errors": _runtime_errors.slice(runtime_error_count_before).duplicate(true),
		}
	return {
		"method": method,
		"ok": true,
		"value": value,
	}


func _normalize_event_stream_value(
	value: Variant,
	sample_index: int,
	elapsed_frames: int,
	sample_frame: int,
	sample_time_msec: int,
	max_events_per_sample: int
) -> Dictionary:
	var raw_events: Array = []
	if value is Array:
		raw_events = value
	elif value is Dictionary:
		var dictionary: Dictionary = value
		if dictionary.has("events"):
			if not dictionary.get("events") is Array:
				return {
					"events": [],
					"event_count": 0,
					"truncated": false,
					"warnings": [_event_trace_warning(
						"event_trace.event_not_json_compatible",
						"event.trace provider returned an events field that is not an array",
						{
							"sample_index": sample_index,
							"elapsed_frames": elapsed_frames,
							"frame": sample_frame,
							"actual_type": type_string(typeof(dictionary.get("events"))),
						},
					)],
				}
			raw_events = dictionary.get("events")
		else:
			raw_events = [dictionary]
	elif value == null:
		raw_events = []
	else:
		return {
			"events": [],
			"event_count": 0,
			"truncated": false,
			"warnings": [_event_trace_warning(
				"event_trace.event_not_json_compatible",
				"event.trace provider returned a value that is not a dictionary or array",
				{
					"sample_index": sample_index,
					"elapsed_frames": elapsed_frames,
					"frame": sample_frame,
					"actual_type": type_string(typeof(value)),
				},
			)],
		}

	var returned_count := mini(raw_events.size(), max_events_per_sample)
	var events: Array = []
	var warnings: Array = []
	for index in range(returned_count):
		var normalized := _normalize_event_record(raw_events[index], sample_index, elapsed_frames, sample_frame, sample_time_msec, index)
		if bool(normalized.get("ok", false)):
			events.append(normalized.get("event", {}))
		else:
			warnings.append(normalized.get("warning", {}))
	if raw_events.size() > returned_count:
		warnings.append(_event_trace_warning(
			"event_trace.event_stream_truncated",
			"event.trace provider returned more events than max_events_per_sample",
			{
				"sample_index": sample_index,
				"elapsed_frames": elapsed_frames,
				"frame": sample_frame,
				"event_count": raw_events.size(),
				"returned_event_count": returned_count,
				"max_events_per_sample": max_events_per_sample,
			},
		))
	return {
		"events": events,
		"event_count": raw_events.size(),
		"truncated": raw_events.size() > returned_count,
		"warnings": warnings,
	}


func _normalize_event_record(value: Variant, sample_index: int, elapsed_frames: int, sample_frame: int, sample_time_msec: int, event_index: int) -> Dictionary:
	if not _is_json_compatible_project_state_value(value):
		return {
			"ok": false,
			"warning": _event_trace_warning(
				"event_trace.event_not_json_compatible",
				"event.trace provider returned a non JSON-compatible event record",
				{
					"sample_index": sample_index,
					"elapsed_frames": elapsed_frames,
					"frame": sample_frame,
					"event_index": event_index,
					"actual_type": type_string(typeof(value)),
				},
			),
		}
	var event: Variant = _json_compatible_value(value)
	if event is Dictionary:
		var event_payload: Dictionary = event
		event_payload["_qa_sample"] = {
			"sample_index": sample_index,
			"elapsed_frames": elapsed_frames,
			"frame": sample_frame,
			"time_msec": sample_time_msec,
			"event_index": event_index,
		}
		return {"ok": true, "event": event_payload}
	return {
		"ok": false,
		"warning": _event_trace_warning(
			"event_trace.event_not_json_compatible",
			"event.trace event records must be dictionaries",
			{
				"sample_index": sample_index,
				"elapsed_frames": elapsed_frames,
				"frame": sample_frame,
				"event_index": event_index,
				"actual_type": type_string(typeof(value)),
			},
		),
	}


func _event_provider_warning_id(status: String) -> String:
	match status:
		"missing_method":
			return "event_trace.provider_missing"
		"rejected_method", "requires_arguments":
			return "event_trace.provider_invalid"
		"runtime_error":
			return "event_trace.provider_error"
		_:
			return "event_trace.provider_error"


func _event_trace_warning(warning_id: String, message: String, data := {}) -> Dictionary:
	var warning := {"id": warning_id, "message": message}
	if data is Dictionary and not (data as Dictionary).is_empty():
		warning["data"] = (data as Dictionary).duplicate(true)
	return warning


func _event_trace_viewport_metadata() -> Dictionary:
	if _scene_root == null or _scene_root.get_viewport() == null:
		return {"available": false}
	var rect := _scene_root.get_viewport().get_visible_rect()
	return {
		"available": true,
		"rect": _rect2_payload(rect),
		"size": _vector2_payload(rect.size),
	}


func _trace_capture_sample(
	resolved_targets: Array,
	params: Dictionary,
	sample_index: int,
	elapsed_frames: int,
	include_physics: bool,
	radius: float
) -> Dictionary:
	var targets: Array = []
	for entry in resolved_targets:
		if not entry is Dictionary:
			continue
		targets.append(_trace_target_sample(entry as Dictionary, params, include_physics, radius))
	return {
		"index": sample_index,
		"elapsed_frames": elapsed_frames,
		"frame": Engine.get_process_frames(),
		"time_msec": Time.get_ticks_msec(),
		"targets": targets,
	}


func _trace_target_sample(entry: Dictionary, params: Dictionary, include_physics: bool, radius: float) -> Dictionary:
	var target := str(entry.get("target", ""))
	var resolution: Dictionary = entry.get("resolution", {})
	var node := entry.get("node") as Node
	var position := _global_position_vector2_for_node(node)
	var visual_rect := _visual_rect_for_node(node)
	var target_info := _physics_target_info(target, resolution, node, position, visual_rect)
	var payload := target_info.duplicate(true)
	if bool(params.get("include_project_state", false)):
		payload["project_state"] = _collect_physics_project_state(node, params, true)
	if include_physics:
		payload["physics"] = _trace_physics_summary(node, position, visual_rect, params, radius)
	return payload


func _trace_physics_summary(
	target_node: Node,
	target_position: Vector2,
	target_visual_rect: Dictionary,
	params: Dictionary,
	radius: float
) -> Dictionary:
	var collision_objects: Array[Dictionary] = []
	var target_shapes: Array[Dictionary] = []
	_collect_collision_object_reports(_scene_root, target_node, target_position, radius, target_visual_rect, collision_objects, target_shapes)
	var space_queries := _physics_space_queries(target_node, target_position, radius, target_shapes)
	var contacts := _collect_physics_contacts(collision_objects, true, target_node, space_queries)
	var target_info := _physics_target_info("", {"resolved": "path:%s" % str(target_node.get_path())}, target_node, target_position, target_visual_rect)
	var project_state := _collect_physics_project_state(
		target_node,
		params,
		bool(params.get("include_project_state", false)),
	)
	var diagnostics := _physics_diagnostics(target_info, collision_objects, target_shapes, project_state)
	return {
		"radius": radius,
		"nearby_collision_object_count": collision_objects.size(),
		"target_shape_count": target_shapes.size(),
		"contact_count": contacts.size(),
		"radius_query_hit_count": int((space_queries.get("radius_query", {}) as Dictionary).get("hit_count", 0)) if space_queries.get("radius_query", {}) is Dictionary else 0,
		"target_shape_overlap_count": _trace_overlap_count(space_queries),
		"contacts": contacts,
		"hidden_collision_objects": _trace_diagnostic_targets(diagnostics, "hidden_collision_object"),
		"layer_mask_mismatches": _trace_diagnostic_targets(diagnostics, "layer_mask_cannot_collide"),
		"diagnostics": diagnostics,
		"project_state": project_state,
	}


func _trace_target_summaries(resolved_targets: Array) -> Array:
	var summaries: Array = []
	for entry in resolved_targets:
		if not entry is Dictionary:
			continue
		var dictionary: Dictionary = entry
		var node := dictionary.get("node") as Node
		summaries.append({
			"target": str(dictionary.get("target", "")),
			"resolved": str((dictionary.get("resolution", {}) as Dictionary).get("resolved", "")),
			"path": str(node.get_path()) if node != null else "",
			"type": node.get_class() if node != null else "",
			"qa_id": str(node.get_meta("qa_id", "")) if node != null else "",
		})
	return summaries


func _trace_derive_events(previous_sample: Dictionary, sample: Dictionary) -> Array:
	if previous_sample.is_empty():
		return []
	var events: Array = []
	for current_target in sample.get("targets", []):
		if not current_target is Dictionary:
			continue
		var current: Dictionary = current_target
		var previous := _trace_find_target_sample(previous_sample, str(current.get("target", "")))
		if previous.is_empty():
			continue
		events.append_array(_trace_motion_events(previous, current, sample))
		events.append_array(_trace_physics_events(previous, current, sample))
		if _trace_project_state_changed(previous, current):
			events.append(_trace_event("project_state_changed", current, sample, {
				"previous_elapsed_frames": previous_sample.get("elapsed_frames", null),
			}))
	return events


func _trace_motion_events(previous: Dictionary, current: Dictionary, sample: Dictionary) -> Array:
	var events: Array = []
	var previous_position := _vector2_from_payload(previous.get("position", {}), Vector2.ZERO)
	var current_position := _vector2_from_payload(current.get("position", {}), Vector2.ZERO)
	var previous_velocity := _trace_velocity_vector(previous)
	var current_velocity := _trace_velocity_vector(current)
	var displacement := current_position - previous_position
	if previous_velocity.length() > 1.0 and displacement.dot(previous_velocity.normalized()) < -1.0:
		events.append(_trace_event("large_backward_displacement", current, sample, {
			"displacement": _vector2_payload(displacement),
			"previous_velocity": _vector2_payload(previous_velocity),
		}))
	if previous_velocity.length() > 1.0 and current_velocity.length() > 1.0 and previous_velocity.dot(current_velocity) < 0.0:
		events.append(_trace_event("velocity_sign_reversal", current, sample, {
			"previous_velocity": _vector2_payload(previous_velocity),
			"current_velocity": _vector2_payload(current_velocity),
		}))
	var previous_speed := previous_velocity.length()
	var current_speed := current_velocity.length()
	if current_speed > previous_speed * 2.0 and current_speed - previous_speed > 50.0:
		events.append(_trace_event("sudden_speed_spike", current, sample, {
			"previous_speed": previous_speed,
			"current_speed": current_speed,
		}))
	return events


func _trace_physics_events(previous: Dictionary, current: Dictionary, sample: Dictionary) -> Array:
	var events: Array = []
	var previous_physics: Dictionary = previous.get("physics", {}) if previous.get("physics", {}) is Dictionary else {}
	var current_physics: Dictionary = current.get("physics", {}) if current.get("physics", {}) is Dictionary else {}
	if current_physics.is_empty():
		return events
	var previous_overlap_count := int(previous_physics.get("target_shape_overlap_count", 0))
	var current_overlap_count := int(current_physics.get("target_shape_overlap_count", 0))
	if previous_overlap_count == 0 and current_overlap_count > 0:
		events.append(_trace_event("new_overlap", current, sample, {
			"previous_overlap_count": previous_overlap_count,
			"current_overlap_count": current_overlap_count,
		}))
	var hidden_colliders: Array = current_physics.get("hidden_collision_objects", []) if current_physics.get("hidden_collision_objects", []) is Array else []
	if not hidden_colliders.is_empty():
		events.append(_trace_event("hidden_collider_nearby", current, sample, {
			"colliders": hidden_colliders,
		}))
	var layer_mismatches: Array = current_physics.get("layer_mask_mismatches", []) if current_physics.get("layer_mask_mismatches", []) is Array else []
	if not layer_mismatches.is_empty():
		events.append(_trace_event("layer_mask_mismatch", current, sample, {
			"objects": layer_mismatches,
		}))
	return events


func _trace_event(event_type: String, target_sample: Dictionary, sample: Dictionary, details: Dictionary) -> Dictionary:
	return {
		"type": event_type,
		"target": target_sample.get("target", ""),
		"resolved": target_sample.get("resolved", ""),
		"frame": sample.get("frame", 0),
		"elapsed_frames": sample.get("elapsed_frames", 0),
		"time_msec": sample.get("time_msec", 0),
		"details": details,
	}


func _trace_find_target_sample(sample: Dictionary, target: String) -> Dictionary:
	for entry in sample.get("targets", []):
		if entry is Dictionary and str((entry as Dictionary).get("target", "")) == target:
			return entry as Dictionary
	return {}


func _trace_velocity_vector(sample: Dictionary) -> Vector2:
	return _vector2_from_payload(sample.get("velocity", {}), Vector2.ZERO)


func _trace_overlap_count(space_queries: Dictionary) -> int:
	var count := 0
	for overlap in space_queries.get("target_shape_overlaps", []):
		if overlap is Dictionary:
			var hits: Array = (overlap as Dictionary).get("hits", []) if (overlap as Dictionary).get("hits", []) is Array else []
			count += hits.size()
	return count


func _trace_diagnostic_targets(diagnostics: Array, diagnostic_type: String) -> Array:
	var targets: Array = []
	for diagnostic in diagnostics:
		if diagnostic is Dictionary and str((diagnostic as Dictionary).get("type", "")) == diagnostic_type:
			targets.append((diagnostic as Dictionary).get("target", ""))
	return targets


func _trace_project_state_changed(previous: Dictionary, current: Dictionary) -> bool:
	var previous_state := _trace_project_state_payload(previous)
	var current_state := _trace_project_state_payload(current)
	if not bool(previous_state.get("available", false)) or not bool(current_state.get("available", false)):
		return false
	return JSON.stringify(previous_state) != JSON.stringify(current_state)


func _trace_project_state_payload(sample: Dictionary) -> Dictionary:
	if sample.get("project_state", {}) is Dictionary:
		return sample.get("project_state", {}) as Dictionary
	var physics: Dictionary = sample.get("physics", {}) if sample.get("physics", {}) is Dictionary else {}
	if physics.get("project_state", {}) is Dictionary:
		return physics.get("project_state", {}) as Dictionary
	return {}


func _trace_event_counts(events: Array) -> Dictionary:
	var counts := {}
	for event in events:
		if not event is Dictionary:
			continue
		var event_type := str((event as Dictionary).get("type", "unknown"))
		counts[event_type] = int(counts.get(event_type, 0)) + 1
	return counts


func _physics_inspect(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", ""))
	var resolution := resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var target_node := resolution.get("node") as Node
	var radius := clampf(float(params.get("radius", 256.0)), 1.0, 4096.0)
	var frames := clampi(int(params.get("frames", 1)), 1, 240)
	if frames > 1:
		for _index in range(frames - 1):
			await _process_frames(1)

	var target_position := _global_position_vector2_for_node(target_node)
	var target_visual_rect := _visual_rect_for_node(target_node)
	var target_info := _physics_target_info(target, resolution, target_node, target_position, target_visual_rect)
	var collision_objects: Array[Dictionary] = []
	var target_shapes: Array[Dictionary] = []
	_collect_collision_object_reports(_scene_root, target_node, target_position, radius, target_visual_rect, collision_objects, target_shapes)
	var space_queries := _physics_space_queries(target_node, target_position, radius, target_shapes)
	var project_state := _collect_physics_project_state(
		target_node,
		params,
		bool(params.get("include_project_state", true)),
	)
	var diagnostics := _physics_diagnostics(target_info, collision_objects, target_shapes, project_state)
	return _ok({
		"target": target,
		"resolved": str(resolution.get("resolved", "")),
		"frame": Engine.get_process_frames(),
		"time_msec": Time.get_ticks_msec(),
		"sample": {
			"frames": frames,
			"radius": radius,
		},
		"target_node": target_info,
		"collision_objects": collision_objects,
		"target_shapes": target_shapes,
		"space_queries": space_queries,
		"project_state": project_state,
		"diagnostics": diagnostics,
	})


func _physics_overlay_collect(params: Dictionary) -> Dictionary:
	var viewport_wide := bool(params.get("viewport", false))
	var target := str(params.get("target", "")).strip_edges()
	var target_node: Node = _scene_root
	var resolution := {
		"ok": true,
		"resolved": "scene-root",
		"node": _scene_root,
	}
	if not viewport_wide or target != "":
		if target == "":
			return _error(
				"invalid_params",
				"physics.overlay.collect requires target unless viewport collection is enabled",
				{
					"command": "physics.overlay.collect",
					"field": "target",
					"expected": "non-empty target locator or viewport=true",
					"actual": params.get("target", null),
				},
			)
		resolution = resolve_target(target)
		if not bool(resolution.get("ok", false)):
			return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
		target_node = resolution.get("node") as Node
	if target_node == null:
		return _error(
			"scene_unavailable",
			"physics.overlay.collect target is not available",
			{
				"command": "physics.overlay.collect",
				"target": target,
			},
		)

	var radius := clampf(float(params.get("radius", 256.0)), 1.0, 4096.0)
	if viewport_wide and not params.has("radius"):
		radius = 4096.0
	var frames := clampi(int(params.get("frames", 1)), 1, 240)
	if frames > 1:
		for _index in range(frames - 1):
			await _process_frames(1)

	var target_position := _global_position_vector2_for_node(target_node)
	var target_visual_rect := _visual_rect_for_node(target_node)
	var collision_objects: Array[Dictionary] = []
	var target_shapes: Array[Dictionary] = []
	var collection_metadata := {
		"include_screen": true,
		"max_shapes": clampi(int(params.get("max_shapes", 512)), 1, 4096),
		"shape_count_total": 0,
		"shape_count_returned": 0,
		"shapes_truncated": false,
	}
	_collect_collision_object_reports(
		_scene_root,
		target_node,
		target_position,
		radius,
		target_visual_rect,
		collision_objects,
		target_shapes,
		collection_metadata,
	)
	var overlay_shapes := _overlay_shape_entries(collision_objects, bool(params.get("include_aabb", true)))
	var project_state := _collect_physics_project_state(
		target_node,
		params,
		bool(params.get("include_project_state", true)),
	)
	var contacts := _collect_physics_contacts(collision_objects, bool(params.get("include_contacts", false)))
	var warnings: Array[Dictionary] = []
	if overlay_shapes.is_empty():
		warnings.append({
			"id": "physics_overlay.no_collision_shapes_found",
			"severity": "warning",
			"target": target,
			"radius": radius,
		})
	if bool(collection_metadata.get("shapes_truncated", false)):
		warnings.append({
			"id": "physics_overlay.unbounded_shape_count_truncated",
			"severity": "warning",
			"max_shapes": collection_metadata.get("max_shapes", 0),
			"shape_count_total": collection_metadata.get("shape_count_total", 0),
			"shape_count_returned": collection_metadata.get("shape_count_returned", 0),
		})

	return _ok({
		"command": "physics.overlay.collect",
		"target": target,
		"viewport": viewport_wide,
		"resolved": str(resolution.get("resolved", "")),
		"frame": Engine.get_process_frames(),
		"time_msec": Time.get_ticks_msec(),
		"sample": {
			"frames": frames,
			"radius": radius,
		},
		"metadata": {
			"shape_count": overlay_shapes.size(),
			"shape_count_total": int(collection_metadata.get("shape_count_total", overlay_shapes.size())),
			"shape_count_returned": int(collection_metadata.get("shape_count_returned", overlay_shapes.size())),
			"truncated": bool(collection_metadata.get("shapes_truncated", false)),
			"max_shapes": int(collection_metadata.get("max_shapes", 0)),
			"body_count": _overlay_object_count(collision_objects, false),
			"area_count": _overlay_object_count(collision_objects, true),
			"contact_count": contacts.size(),
			"warnings": warnings,
		},
		"collision_objects": collision_objects,
		"shapes": overlay_shapes,
		"contacts": contacts,
		"project_state": project_state,
	})


func _physics_target_info(target: String, resolution: Dictionary, node: Node, position: Vector2, visual_rect: Dictionary) -> Dictionary:
	var payload := {
		"target": target,
		"resolved": str(resolution.get("resolved", "")),
		"path": str(node.get_path()),
		"name": str(node.name),
		"type": node.get_class(),
		"qa_id": str(node.get_meta("qa_id", "")),
		"position": _vector2_payload(position),
		"transform": _node_transform_payload(node),
		"rotation": rotation_for_node(node),
		"velocity": _physics_velocity_payload(node),
		"angular_velocity": _physics_angular_velocity_payload(node),
		"visible": _node_visible_for_physics(node),
		"visual_rect": visual_rect,
		"is_collision_object_2d": node is CollisionObject2D,
	}
	if node is CollisionObject2D:
		var collision_object := node as CollisionObject2D
		payload["collision_layer"] = collision_object.collision_layer
		payload["collision_mask"] = collision_object.collision_mask
	return payload


func _collect_collision_object_reports(
	root: Node,
	target_node: Node,
	target_position: Vector2,
	radius: float,
	target_visual_rect: Dictionary,
	reports: Array,
	target_shapes: Array,
	collection_options := {}
) -> void:
	if root == null:
		return
	if root is CollisionObject2D:
		var collision_object := root as CollisionObject2D
		var object_position := _global_position_vector2_for_node(collision_object)
		var object_shapes := _collision_object_shapes_report(collision_object, target_position, target_visual_rect, collection_options)
		var nearest_distance := target_position.distance_to(object_position)
		for shape_report in object_shapes:
			if shape_report is Dictionary:
				nearest_distance = minf(nearest_distance, float((shape_report as Dictionary).get("distance_to_target", nearest_distance)))
		if nearest_distance <= radius or root == target_node:
			var reported_shapes := _bounded_collision_object_shapes(object_shapes, collection_options)
			var object_report := {
				"path": str(collision_object.get_path()),
				"name": str(collision_object.name),
				"type": collision_object.get_class(),
				"qa_id": str(collision_object.get_meta("qa_id", "")),
				"position": _vector2_payload(object_position),
				"visible": _node_visible_for_physics(collision_object),
				"collision_layer": collision_object.collision_layer,
				"collision_mask": collision_object.collision_mask,
				"distance_to_target": nearest_distance,
				"is_target": root == target_node,
				"is_area": collision_object is Area2D,
				"is_body": collision_object is PhysicsBody2D,
				"shapes": reported_shapes,
			}
			reports.append(object_report)
			if root == target_node:
				for shape_report in reported_shapes:
					target_shapes.append((shape_report as Dictionary).duplicate(true))
	for child in root.get_children():
		if child is Node:
			_collect_collision_object_reports(child as Node, target_node, target_position, radius, target_visual_rect, reports, target_shapes, collection_options)


func _collision_object_shapes_report(collision_object: CollisionObject2D, target_position: Vector2, target_visual_rect: Dictionary, collection_options := {}) -> Array[Dictionary]:
	var shapes: Array[Dictionary] = []
	var options: Dictionary = collection_options if collection_options is Dictionary else {}
	var include_screen := bool(options.get("include_screen", false))
	for owner_id in collision_object.get_shape_owners():
		var owner = collision_object.shape_owner_get_owner(owner_id)
		var owner_path := ""
		if owner is Node:
			owner_path = str((owner as Node).get_path())
		var owner_transform: Transform2D = collision_object.global_transform * collision_object.shape_owner_get_transform(owner_id)
		var disabled := false
		if collision_object.has_method("is_shape_owner_disabled"):
			disabled = bool(collision_object.call("is_shape_owner_disabled", owner_id))
		var one_way := false
		if collision_object.has_method("shape_owner_get_one_way_collision"):
			one_way = bool(collision_object.call("shape_owner_get_one_way_collision", owner_id))
		var shape_count := collision_object.shape_owner_get_shape_count(owner_id)
		for shape_index in range(shape_count):
			var shape := collision_object.shape_owner_get_shape(owner_id, shape_index)
			var aabb := _shape2d_aabb_payload(shape, owner_transform)
			var shape_payload := {
				"node_path": str(collision_object.get_path()),
				"node_type": collision_object.get_class(),
				"qa_id": str(collision_object.get_meta("qa_id", "")),
				"layer": collision_object.collision_layer,
				"mask": collision_object.collision_mask,
				"is_area": collision_object is Area2D,
				"is_body": collision_object is PhysicsBody2D,
				"is_soft": false,
				"owner_id": owner_id,
				"shape_index": shape_index,
				"shape_type": _shape_type_name(shape),
				"overlay_shape_type": _shape2d_overlay_type_name(shape),
				"owner_path": owner_path,
				"disabled": disabled,
				"one_way_collision": one_way,
				"transform": _transform2d_payload(owner_transform),
				"aabb": aabb,
				"world_aabb": aabb,
				"distance_to_target": _distance_from_rect_to_point(aabb, target_position),
				"large_shape_vs_visual": _shape_large_vs_visual(aabb, target_visual_rect),
			}
			if include_screen:
				var projection := _shape2d_projection_payload(shape, owner_transform, _screen_transform_for_collision_object(collision_object))
				shape_payload["world_points"] = projection.get("world_points", [])
				shape_payload["screen_points"] = projection.get("screen_points", [])
				shape_payload["screen_aabb"] = projection.get("screen_aabb", {})
			shapes.append(shape_payload)
	return shapes


func _bounded_collision_object_shapes(object_shapes: Array[Dictionary], collection_options := {}) -> Array[Dictionary]:
	if not collection_options is Dictionary or (collection_options as Dictionary).is_empty():
		return object_shapes
	var options: Dictionary = collection_options
	options["shape_count_total"] = int(options.get("shape_count_total", 0)) + object_shapes.size()
	var max_shapes := int(options.get("max_shapes", 0))
	if max_shapes <= 0:
		options["shape_count_returned"] = int(options.get("shape_count_returned", 0)) + object_shapes.size()
		return object_shapes
	var remaining := max_shapes - int(options.get("shape_count_returned", 0))
	if remaining <= 0:
		options["shapes_truncated"] = true
		return []
	var returned: Array[Dictionary] = []
	var returned_count := mini(remaining, object_shapes.size())
	for index in range(returned_count):
		returned.append(object_shapes[index])
	options["shape_count_returned"] = int(options.get("shape_count_returned", 0)) + returned.size()
	if returned.size() < object_shapes.size():
		options["shapes_truncated"] = true
	return returned


func _physics_space_queries(target_node: Node, target_position: Vector2, radius: float, target_shapes: Array) -> Dictionary:
	var world := _world_2d_for_node(target_node)
	if world == null:
		return {"available": false, "reason": "world_2d_unavailable"}
	var space_state := world.direct_space_state
	var radius_shape := CircleShape2D.new()
	radius_shape.radius = radius
	var radius_query := PhysicsShapeQueryParameters2D.new()
	radius_query.shape = radius_shape
	radius_query.transform = Transform2D(0.0, target_position)
	radius_query.collision_mask = 0xFFFFFFFF
	radius_query.collide_with_areas = true
	radius_query.collide_with_bodies = true
	var radius_hits := _serialize_intersect_shape_results(space_state.intersect_shape(radius_query, 64), target_node)
	var target_overlap_hits: Array = []
	for shape_report in target_shapes:
		if not shape_report is Dictionary:
			continue
		var source_shape := _shape_from_target_report(target_node, shape_report as Dictionary)
		if source_shape == null:
			continue
		var target_query := PhysicsShapeQueryParameters2D.new()
		target_query.shape = source_shape
		target_query.transform = _transform2d_from_payload((shape_report as Dictionary).get("transform", {}))
		target_query.collision_mask = 0xFFFFFFFF
		target_query.collide_with_areas = true
		target_query.collide_with_bodies = true
		if target_node is CollisionObject2D:
			target_query.exclude = [(target_node as CollisionObject2D).get_rid()]
		target_overlap_hits.append({
			"owner_id": (shape_report as Dictionary).get("owner_id", null),
			"shape_index": (shape_report as Dictionary).get("shape_index", null),
			"hits": _serialize_intersect_shape_results(space_state.intersect_shape(target_query, 32), target_node),
		})
	return {
		"available": true,
		"radius_query": {
			"radius": radius,
			"hit_count": radius_hits.size(),
			"hits": radius_hits,
		},
		"target_shape_overlaps": target_overlap_hits,
	}


func _collect_physics_project_state(target_node: Node, params: Dictionary, include_project_state: bool) -> Dictionary:
	if not include_project_state:
		return {"enabled": false, "available": false, "providers": []}
	var raw_methods: Array = params.get("state_methods", []) if params.get("state_methods", []) is Array else []
	var methods: Array[String] = []
	if raw_methods.is_empty():
		methods = [
			"get_qa_collision_state",
			"get_collision_qa_state",
			"get_qa_physics_state",
			"get_physics_qa_state",
			"qa_collision_probe",
			"get_qa_collision_footprint_state",
			"get_qa_visual_footprint_state",
			"get_qa_collision_visual_state",
		]
	else:
		for raw_method in raw_methods:
			var method := str(raw_method)
			if _is_allowed_physics_state_method(method):
				methods.append(method)
	var providers: Array[Node] = []
	if raw_methods.is_empty():
		if _scene_root != null:
			providers.append(_scene_root)
		if target_node != null and target_node != _scene_root:
			providers.append(target_node)
	else:
		var provider_locator := str(params.get("state_provider", "scene-root"))
		if provider_locator == "scene-root":
			if _scene_root != null:
				providers.append(_scene_root)
		else:
			var provider_resolution := resolve_target(provider_locator)
			if bool(provider_resolution.get("ok", false)):
				providers.append(provider_resolution.get("node") as Node)
			else:
				return {
					"enabled": true,
					"available": false,
					"providers": [],
					"error": provider_resolution.get("error", {}),
				}
	var provider_payloads: Array = []
	var available := false
	for provider in providers:
		var methods_payload: Array = []
		for method in methods:
			var method_payload := _call_physics_state_method(provider, method)
			method_payload = _bounded_visual_footprint_method_payload(method_payload, params)
			methods_payload.append(method_payload)
			available = available or bool(method_payload.get("ok", false))
		provider_payloads.append({
			"resolved": "path:%s" % str(provider.get_path()),
			"path": str(provider.get_path()),
			"type": provider.get_class(),
			"methods": methods_payload,
		})
	return {
		"enabled": true,
		"available": available,
		"providers": provider_payloads,
	}


func _bounded_visual_footprint_method_payload(method_payload: Dictionary, params: Dictionary) -> Dictionary:
	if not bool(method_payload.get("ok", false)):
		return method_payload
	var value: Variant = method_payload.get("value", null)
	if not value is Dictionary:
		return method_payload
	var dictionary: Dictionary = value
	if not dictionary.has("visual_footprints") or not dictionary.get("visual_footprints") is Array:
		return method_payload
	var max_entries := clampi(int(params.get("max_shapes", 512)), 1, 4096)
	var footprints: Array = dictionary.get("visual_footprints")
	var bounded: Array = []
	var returned_count := mini(footprints.size(), max_entries)
	for index in range(returned_count):
		bounded.append(footprints[index])
	var next_value := dictionary.duplicate(true)
	next_value["visual_footprints"] = bounded
	next_value["visual_footprint_count"] = footprints.size()
	next_value["visual_footprint_returned_count"] = bounded.size()
	next_value["visual_footprints_truncated"] = footprints.size() > bounded.size()
	method_payload = method_payload.duplicate(true)
	method_payload["value"] = next_value
	method_payload["visual_footprint_count"] = footprints.size()
	method_payload["visual_footprint_returned_count"] = bounded.size()
	method_payload["visual_footprints_truncated"] = footprints.size() > bounded.size()
	return method_payload


func _overlay_shape_entries(collision_objects: Array, include_aabb: bool) -> Array[Dictionary]:
	var entries: Array[Dictionary] = []
	for object_report in collision_objects:
		if not object_report is Dictionary:
			continue
		var object_payload: Dictionary = object_report
		for shape in object_payload.get("shapes", []):
			if not shape is Dictionary:
				continue
			var shape_payload: Dictionary = shape
			var entry := {
				"logical_id": _overlay_logical_id(object_payload, shape_payload),
				"qa_id": str(object_payload.get("qa_id", "")),
				"node_path": str(object_payload.get("path", shape_payload.get("node_path", ""))),
				"node_type": str(object_payload.get("type", shape_payload.get("node_type", ""))),
				"shape_type": str(shape_payload.get("overlay_shape_type", shape_payload.get("shape_type", "unknown"))),
				"layer": int(object_payload.get("collision_layer", shape_payload.get("layer", 0))),
				"mask": int(object_payload.get("collision_mask", shape_payload.get("mask", 0))),
				"owner_id": shape_payload.get("owner_id", null),
				"shape_index": shape_payload.get("shape_index", null),
				"disabled": bool(shape_payload.get("disabled", false)),
				"world_aabb": shape_payload.get("world_aabb", shape_payload.get("aabb", {})),
				"screen_aabb": shape_payload.get("screen_aabb", {}),
				"screen_points": shape_payload.get("screen_points", []),
				"is_area": bool(object_payload.get("is_area", false)),
				"is_body": bool(object_payload.get("is_body", false)),
				"is_soft": bool(shape_payload.get("is_soft", false)),
			}
			if include_aabb:
				entry["aabb"] = shape_payload.get("aabb", {})
			entries.append(entry)
	return entries


func _overlay_logical_id(object_payload: Dictionary, shape_payload: Dictionary) -> String:
	var qa_id := str(object_payload.get("qa_id", ""))
	var base := qa_id if qa_id != "" else str(object_payload.get("path", shape_payload.get("node_path", "")))
	return "%s#%s:%s" % [base, str(shape_payload.get("owner_id", "")), str(shape_payload.get("shape_index", ""))]


func _overlay_object_count(collision_objects: Array, count_areas: bool) -> int:
	var count := 0
	for object_report in collision_objects:
		if not object_report is Dictionary:
			continue
		var report: Dictionary = object_report
		if count_areas and bool(report.get("is_area", false)):
			count += 1
		elif not count_areas and bool(report.get("is_body", false)):
			count += 1
	return count


func _collect_physics_contacts(
	collision_objects: Array,
	include_contacts: bool,
	target_node: Node = null,
	space_queries: Dictionary = {}
) -> Array[Dictionary]:
	var contacts: Array[Dictionary] = []
	if not include_contacts:
		return contacts
	for object_report in collision_objects:
		if not object_report is Dictionary:
			continue
		if target_node != null and not bool((object_report as Dictionary).get("is_target", false)):
			continue
		var path := str((object_report as Dictionary).get("path", ""))
		if path == "" or _scene_root == null:
			continue
		var node := _scene_root.get_node_or_null(NodePath(path))
		if node == null:
			continue
		if node.has_method("get_contact_count"):
			var count := int(node.call("get_contact_count"))
			for index in range(count):
				var contact := {
					"node_path": path,
					"index": index,
					"kind": "body_contact",
					"available": true,
				}
				if node.has_method("get_contact_local_normal"):
					contact["normal"] = _vector2_payload(node.call("get_contact_local_normal", index))
				if node.has_method("get_contact_local_position"):
					contact["position"] = _vector2_payload(node.call("get_contact_local_position", index))
				if node.has_method("get_contact_impulse"):
					contact["impulse"] = _json_compatible_value(node.call("get_contact_impulse", index))
				if node.has_method("get_contact_collider_object"):
					var collider = node.call("get_contact_collider_object", index)
					contact.merge(_contact_collider_payload(collider), true)
				elif node.has_method("get_contact_collider"):
					contact["collider_rid"] = str(node.call("get_contact_collider", index))
				if node.has_method("get_contact_collider_id"):
					contact["collider_instance_id"] = int(node.call("get_contact_collider_id", index))
				if node.has_method("get_contact_collider_shape"):
					contact["collider_shape"] = int(node.call("get_contact_collider_shape", index))
				if node.has_method("get_contact_local_shape"):
					contact["local_shape"] = int(node.call("get_contact_local_shape", index))
				contact["contact_id"] = _contact_id_for_payload(node, contact, index)
				contacts.append(contact)
		elif node is CharacterBody2D and node.has_method("get_slide_collision_count"):
			var slide_count := int(node.call("get_slide_collision_count"))
			for index in range(slide_count):
				var collision = node.call("get_slide_collision", index)
				var contact := {
					"node_path": path,
					"index": index,
					"kind": "slide_collision",
					"available": true,
				}
				if collision != null:
					if collision.has_method("get_position"):
						contact["position"] = _vector2_payload(collision.call("get_position"))
					if collision.has_method("get_normal"):
						contact["normal"] = _vector2_payload(collision.call("get_normal"))
					if collision.has_method("get_depth"):
						contact["depth"] = float(collision.call("get_depth"))
					if collision.has_method("get_collider"):
						contact.merge(_contact_collider_payload(collision.call("get_collider")), true)
					if collision.has_method("get_collider_id"):
						contact["collider_instance_id"] = int(collision.call("get_collider_id"))
					if collision.has_method("get_collider_shape"):
						contact["collider_shape"] = int(collision.call("get_collider_shape"))
					if collision.has_method("get_collider_velocity"):
						contact["collider_velocity"] = _vector2_payload(collision.call("get_collider_velocity"))
					if collision.has_method("get_local_shape"):
						contact["local_shape"] = int(collision.call("get_local_shape"))
				contact["contact_id"] = _contact_id_for_payload(node, contact, index)
				contacts.append(contact)
	contacts.append_array(_collect_target_overlap_contacts(target_node, space_queries))
	return contacts


func _collect_target_overlap_contacts(target_node: Node, space_queries: Dictionary) -> Array[Dictionary]:
	var contacts: Array[Dictionary] = []
	if target_node == null or not space_queries.get("available", false):
		return contacts
	var overlaps: Array = space_queries.get("target_shape_overlaps", []) if space_queries.get("target_shape_overlaps", []) is Array else []
	for overlap_index in range(overlaps.size()):
		if not overlaps[overlap_index] is Dictionary:
			continue
		var overlap: Dictionary = overlaps[overlap_index]
		var hits: Array = overlap.get("hits", []) if overlap.get("hits", []) is Array else []
		for hit_index in range(hits.size()):
			if not hits[hit_index] is Dictionary:
				continue
			var hit: Dictionary = hits[hit_index]
			var contact := {
				"node_path": str(target_node.get_path()),
				"index": hit_index,
				"kind": "shape_overlap",
				"available": true,
				"owner_id": overlap.get("owner_id", null),
				"shape_index": overlap.get("shape_index", null),
				"depth_status": "not_evaluated",
			}
			contact.merge(hit, true)
			contact["contact_id"] = _contact_id_for_payload(target_node, contact, overlap_index * 1000 + hit_index)
			contacts.append(contact)
	return contacts


func _contact_collider_payload(collider: Variant) -> Dictionary:
	if collider is Node:
		var node := collider as Node
		return {
			"collider_path": str(node.get_path()),
			"collider_type": node.get_class(),
			"collider_qa_id": str(node.get_meta("qa_id", "")),
			"other_id": _stable_node_contact_id(node),
		}
	return {"other_id": str(collider)}


func _contact_id_for_payload(node: Node, contact: Dictionary, index: int) -> String:
	var self_id := _stable_node_contact_id(node)
	var other_id := str(contact.get("other_id", ""))
	if other_id == "":
		other_id = str(contact.get("collider_qa_id", ""))
	if other_id == "":
		other_id = str(contact.get("collider_path", ""))
	if other_id == "":
		other_id = str(contact.get("rid", contact.get("collider_rid", "unknown")))
	return "%s->%s#%s" % [self_id, other_id, str(index)]


func _stable_node_contact_id(node: Node) -> String:
	if node == null:
		return ""
	var qa_id := str(node.get_meta("qa_id", ""))
	if qa_id != "":
		return "qa:%s" % qa_id
	return "path:%s" % str(node.get_path())


func _trace_project_state_preflight(params: Dictionary) -> Dictionary:
	if not bool(params.get("include_project_state", false)):
		return {"ok": true}
	var raw_methods: Array = params.get("state_methods", []) if params.get("state_methods", []) is Array else []
	if raw_methods.is_empty():
		return {"ok": true}
	var methods: Array[String] = []
	for raw_method in raw_methods:
		var method := str(raw_method)
		if not _is_allowed_physics_state_method(method):
			return _error(
				"invalid_params",
				"trace.record state method is not an approved read-only QA state method",
				{
					"command": "trace.record",
					"field": "state_methods",
					"method": method,
					"expected": "get_qa_*_state, get_*_qa_state, or qa_*diagnostic*",
					"actual": raw_method,
				},
			)
		methods.append(method)
	var provider_result := _trace_project_state_provider(params)
	if not bool(provider_result.get("ok", false)):
		return provider_result
	var provider := provider_result.get("node") as Node
	for method in methods:
		var method_payload := _call_physics_state_method(provider, method)
		if not bool(method_payload.get("ok", false)):
			var status := str(method_payload.get("status", "invalid_state_method"))
			return _error(
				_trace_state_method_failure_type(status),
				_trace_state_method_failure_message(status),
				{
					"command": "trace.record",
					"state_provider": str(params.get("state_provider", "scene-root")),
					"provider": provider_result.get("resolved", ""),
					"method": method,
					"status": status,
					"method_result": method_payload,
				},
			)
	return {"ok": true}


func _trace_project_state_provider(params: Dictionary) -> Dictionary:
	var provider_locator := str(params.get("state_provider", "scene-root")).strip_edges()
	if provider_locator == "":
		return _error(
			"invalid_params",
			"trace.record requires state_provider to be non-empty",
			{
				"command": "trace.record",
				"field": "state_provider",
				"expected": "scene-root or target locator",
				"actual": params.get("state_provider", null),
			},
		)
	if provider_locator == "scene-root":
		if _scene_root == null:
			return _error(
				"scene_unavailable",
				"trace.record state provider scene-root is not available",
				{
					"command": "trace.record",
					"state_provider": provider_locator,
					"expected": "active scene root",
					"actual": null,
				},
			)
		return {
			"ok": true,
			"node": _scene_root,
			"resolved": "scene-root",
			"path": str(_scene_root.get_path()),
		}
	var provider_resolution := resolve_target(provider_locator)
	if not bool(provider_resolution.get("ok", false)):
		var error_payload: Dictionary = provider_resolution.get("error", {}) if provider_resolution.get("error", {}) is Dictionary else {}
		error_payload = error_payload.duplicate(true)
		var error_data: Dictionary = error_payload.get("data", {}) if error_payload.get("data", {}) is Dictionary else {}
		error_data = error_data.duplicate(true)
		error_data["command"] = "trace.record"
		error_data["state_provider"] = provider_locator
		error_payload["data"] = error_data
		return {"ok": false, "error": error_payload}
	var provider := provider_resolution.get("node") as Node
	if provider == null:
		return _error(
			"target_not_found",
			"trace.record state provider did not resolve to a node",
			{
				"command": "trace.record",
				"state_provider": provider_locator,
				"expected": "resolved provider node",
				"actual": provider_resolution.get("resolved", ""),
			},
		)
	return {
		"ok": true,
		"node": provider,
		"resolved": str(provider_resolution.get("resolved", "path:%s" % str(provider.get_path()))),
		"path": str(provider.get_path()),
	}


func _trace_state_method_failure_type(status: String) -> String:
	match status:
		"missing_method":
			return "missing_state_method"
		"requires_arguments", "rejected_method":
			return "invalid_params"
		"runtime_error":
			return "runtime_errors_present"
		"non_json_return":
			return "invalid_state_source"
		_:
			return "invalid_state_method"


func _trace_state_method_failure_message(status: String) -> String:
	match status:
		"missing_method":
			return "trace.record state provider does not expose the requested QA state method"
		"requires_arguments":
			return "trace.record state methods must take zero arguments"
		"runtime_error":
			return "trace.record state method recorded runtime errors"
		"non_json_return":
			return "trace.record state method returned a non JSON-compatible value"
		"rejected_method":
			return "trace.record state method is not an approved read-only QA state method"
		_:
			return "trace.record state method failed preflight"


func _call_physics_state_method(provider: Node, method: String) -> Dictionary:
	if not _is_allowed_physics_state_method(method):
		return {"method": method, "ok": false, "status": "rejected_method"}
	if not provider.has_method(method):
		return {"method": method, "ok": false, "status": "missing_method"}
	if not _method_has_zero_required_arguments(provider, method):
		return {"method": method, "ok": false, "status": "requires_arguments"}
	var runtime_error_count_before := _runtime_errors.size()
	var value = provider.call(method)
	if _runtime_errors.size() > runtime_error_count_before:
		return {
			"method": method,
			"ok": false,
			"status": "runtime_error",
			"runtime_errors": _runtime_errors.slice(runtime_error_count_before).duplicate(true),
		}
	if not _is_json_compatible_project_state_value(value):
		return {
			"method": method,
			"ok": false,
			"status": "non_json_return",
			"actual_type": type_string(typeof(value)),
		}
	return {
		"method": method,
		"ok": true,
		"value": _json_compatible_value(value),
	}


func _physics_diagnostics(target_info: Dictionary, collision_objects: Array, target_shapes: Array, project_state: Dictionary) -> Array[Dictionary]:
	var diagnostics: Array[Dictionary] = []
	if target_shapes.is_empty():
		diagnostics.append({
			"type": "no_engine_target_shape",
			"severity": "info",
			"message": "target has no CollisionObject2D-owned shapes",
			"target": target_info.get("resolved", target_info.get("path", "")),
		})
	for object_report in collision_objects:
		if not object_report is Dictionary:
			continue
		var report: Dictionary = object_report
		if not bool(report.get("visible", true)):
			diagnostics.append({
				"type": "hidden_collision_object",
				"severity": "warning",
				"message": "collision object is not visible in tree",
				"target": report.get("path", ""),
			})
		if bool(report.get("is_target", false)):
			continue
		if target_info.get("collision_mask", null) != null and report.get("collision_layer", null) != null:
			var target_mask := int(target_info.get("collision_mask", 0))
			var target_layer := int(target_info.get("collision_layer", 0))
			var other_mask := int(report.get("collision_mask", 0))
			var other_layer := int(report.get("collision_layer", 0))
			if (target_mask & other_layer) == 0 or (other_mask & target_layer) == 0:
				diagnostics.append({
					"type": "layer_mask_cannot_collide",
					"severity": "info",
					"message": "target and nearby object layer/mask settings do not mutually collide",
					"target": report.get("path", ""),
				})
		for shape in report.get("shapes", []):
			if shape is Dictionary and bool((shape as Dictionary).get("large_shape_vs_visual", false)):
				diagnostics.append({
					"type": "large_shape_vs_visual",
					"severity": "warning",
					"message": "shape AABB is much larger than the target visual rect",
					"target": report.get("path", ""),
					"shape": {
						"owner_id": (shape as Dictionary).get("owner_id", null),
						"shape_index": (shape as Dictionary).get("shape_index", null),
					},
				})
	if bool(project_state.get("available", false)):
		diagnostics.append({
			"type": "project_state_available",
			"severity": "info",
			"message": "project-owned QA physics/collision state was captured",
		})
	return diagnostics


func _global_position_vector2_for_node(node: Node) -> Vector2:
	if node is Node2D:
		return (node as Node2D).global_position
	if node is Control:
		return (node as Control).global_position
	return Vector2.ZERO


func _physics_velocity_payload(node: Node) -> Variant:
	if node is CharacterBody2D:
		return _vector2_payload((node as CharacterBody2D).velocity)
	if node is RigidBody2D:
		return _vector2_payload((node as RigidBody2D).linear_velocity)
	if node is CharacterBody3D:
		var velocity_3d := (node as CharacterBody3D).velocity
		return {"x": velocity_3d.x, "y": velocity_3d.y, "z": velocity_3d.z, "length": velocity_3d.length()}
	if node is RigidBody3D:
		var rigid_velocity_3d := (node as RigidBody3D).linear_velocity
		return {"x": rigid_velocity_3d.x, "y": rigid_velocity_3d.y, "z": rigid_velocity_3d.z, "length": rigid_velocity_3d.length()}
	return null


func _physics_angular_velocity_payload(node: Node) -> Variant:
	if node is RigidBody2D:
		return (node as RigidBody2D).angular_velocity
	if node is RigidBody3D:
		var angular_velocity_3d := (node as RigidBody3D).angular_velocity
		return {"x": angular_velocity_3d.x, "y": angular_velocity_3d.y, "z": angular_velocity_3d.z, "length": angular_velocity_3d.length()}
	return null


func _node_transform_payload(node: Node) -> Variant:
	if node is Node2D:
		return _transform2d_payload((node as Node2D).global_transform)
	if node is Control:
		return _transform2d_payload((node as Control).get_global_transform())
	if node is Node3D:
		var transform_3d := (node as Node3D).global_transform
		return {
			"origin": {"x": transform_3d.origin.x, "y": transform_3d.origin.y, "z": transform_3d.origin.z},
			"basis_x": {"x": transform_3d.basis.x.x, "y": transform_3d.basis.x.y, "z": transform_3d.basis.x.z},
			"basis_y": {"x": transform_3d.basis.y.x, "y": transform_3d.basis.y.y, "z": transform_3d.basis.y.z},
			"basis_z": {"x": transform_3d.basis.z.x, "y": transform_3d.basis.z.y, "z": transform_3d.basis.z.z},
		}
	return null


func _node_visible_for_physics(node: Node) -> bool:
	if node is CanvasItem:
		return (node as CanvasItem).is_visible_in_tree()
	return true


func _visual_rect_for_node(node: Node) -> Dictionary:
	var explicit_rect := _explicit_visual_rect_for_node(node)
	if not explicit_rect.is_empty():
		return explicit_rect
	if node is Control:
		return _rect2_payload((node as Control).get_global_rect())
	if node is Sprite2D:
		var sprite := node as Sprite2D
		var texture := sprite.texture
		if texture != null:
			var size := texture.get_size() * sprite.global_scale.abs()
			return _rect2_payload(Rect2(sprite.global_position - size * 0.5, size))
	return {}


func _explicit_visual_rect_for_node(node: Node) -> Dictionary:
	var value: Variant = null
	if node.has_method("qa_visual_rect"):
		value = node.call("qa_visual_rect")
	elif node.has_meta("qa_visual_rect"):
		value = node.get_meta("qa_visual_rect")
	return _rect2_from_variant(value)


func _rect2_from_variant(value: Variant) -> Dictionary:
	if value is Rect2:
		return _rect2_payload(value)
	if value is Array:
		var array: Array = value
		if array.size() >= 4 and _is_number(array[0]) and _is_number(array[1]) and _is_number(array[2]) and _is_number(array[3]):
			return _rect2_payload(Rect2(Vector2(float(array[0]), float(array[1])), Vector2(float(array[2]), float(array[3]))))
	if value is Dictionary:
		var dictionary: Dictionary = value
		if dictionary.has("rect"):
			return _rect2_from_variant(dictionary.get("rect"))
		if _is_number(dictionary.get("x")) and _is_number(dictionary.get("y")) and _is_number(dictionary.get("width")) and _is_number(dictionary.get("height")):
			return _rect2_payload(Rect2(Vector2(float(dictionary.get("x")), float(dictionary.get("y"))), Vector2(float(dictionary.get("width")), float(dictionary.get("height")))))
	return {}


func _shape2d_aabb_payload(shape: Shape2D, transform: Transform2D) -> Dictionary:
	var points := _shape2d_local_outline_points(shape)
	if points.is_empty():
		return {}
	return _rect2_payload(_rect2_from_points(_transform_points(points, transform)))


func _shape2d_projection_payload(shape: Shape2D, world_transform: Transform2D, screen_transform: Transform2D) -> Dictionary:
	var local_points := _shape2d_local_outline_points(shape)
	if local_points.is_empty():
		return {
			"world_points": [],
			"screen_points": [],
			"world_aabb": {},
			"screen_aabb": {},
		}
	var world_points := _transform_points(local_points, world_transform)
	var screen_points := _transform_points(local_points, screen_transform * world_transform)
	return {
		"world_points": _vector2_payloads(world_points),
		"screen_points": _vector2_payloads(screen_points),
		"world_aabb": _rect2_payload(_rect2_from_points(world_points)),
		"screen_aabb": _rect2_payload(_rect2_from_points(screen_points)),
	}


func _shape2d_local_outline_points(shape: Shape2D) -> Array[Vector2]:
	var points: Array[Vector2] = []
	if shape is CircleShape2D:
		var radius := (shape as CircleShape2D).radius
		for index in range(24):
			var angle := TAU * float(index) / 24.0
			points.append(Vector2(cos(angle), sin(angle)) * radius)
	elif shape is RectangleShape2D:
		var half_size := (shape as RectangleShape2D).size * 0.5
		points = [Vector2(-half_size.x, -half_size.y), Vector2(half_size.x, -half_size.y), Vector2(half_size.x, half_size.y), Vector2(-half_size.x, half_size.y)]
	elif shape is CapsuleShape2D:
		var capsule := shape as CapsuleShape2D
		var straight_half := maxf(capsule.height * 0.5 - capsule.radius, 0.0)
		for index in range(12):
			var top_angle := PI + PI * float(index) / 11.0
			points.append(Vector2(cos(top_angle) * capsule.radius, -straight_half + sin(top_angle) * capsule.radius))
		for index in range(12):
			var bottom_angle := PI * float(index) / 11.0
			points.append(Vector2(cos(bottom_angle) * capsule.radius, straight_half + sin(bottom_angle) * capsule.radius))
	elif shape is SegmentShape2D:
		var segment := shape as SegmentShape2D
		points = [segment.a, segment.b]
	elif shape is SeparationRayShape2D:
		points = [Vector2.ZERO, Vector2(0.0, (shape as SeparationRayShape2D).length)]
	elif shape is ConvexPolygonShape2D:
		for point in (shape as ConvexPolygonShape2D).points:
			points.append(point)
	elif shape is ConcavePolygonShape2D:
		for point in (shape as ConcavePolygonShape2D).segments:
			points.append(point)
	if points.is_empty() and shape != null:
		points = [Vector2.ZERO]
	return points


func _transform_points(points: Array[Vector2], transform: Transform2D) -> Array[Vector2]:
	var transformed_points: Array[Vector2] = []
	for point in points:
		transformed_points.append(transform * point)
	return transformed_points


func _vector2_payloads(points: Array[Vector2]) -> Array[Dictionary]:
	var payloads: Array[Dictionary] = []
	for point in points:
		payloads.append(_vector2_payload(point))
	return payloads


func _rect2_from_points(points: Array[Vector2]) -> Rect2:
	if points.is_empty():
		return Rect2()
	var min_point := points[0]
	var max_point := points[0]
	for point in points:
		min_point.x = minf(min_point.x, point.x)
		min_point.y = minf(min_point.y, point.y)
		max_point.x = maxf(max_point.x, point.x)
		max_point.y = maxf(max_point.y, point.y)
	return Rect2(min_point, max_point - min_point)


func _screen_transform_for_collision_object(collision_object: CollisionObject2D) -> Transform2D:
	var viewport := collision_object.get_viewport()
	if viewport != null:
		return viewport.get_canvas_transform()
	if _scene_root != null and _scene_root.get_viewport() != null:
		return _scene_root.get_viewport().get_canvas_transform()
	return Transform2D.IDENTITY


func _shape2d_overlay_type_name(shape: Shape2D) -> String:
	if shape is CircleShape2D:
		return "circle"
	if shape is RectangleShape2D:
		return "rect"
	if shape is CapsuleShape2D:
		return "capsule"
	if shape is SegmentShape2D or shape is SeparationRayShape2D:
		return "segment"
	if shape is ConvexPolygonShape2D or shape is ConcavePolygonShape2D:
		return "polygon"
	return "unknown"


func _shape_type_name(shape: Shape2D) -> String:
	if shape == null:
		return "null"
	return shape.get_class()


func _shape_large_vs_visual(shape_aabb: Dictionary, visual_rect: Dictionary) -> bool:
	if shape_aabb.is_empty() or visual_rect.is_empty():
		return false
	var shape_area := maxf(float(shape_aabb.get("width", 0.0)), 0.0) * maxf(float(shape_aabb.get("height", 0.0)), 0.0)
	var visual_area := maxf(float(visual_rect.get("width", 0.0)), 0.0) * maxf(float(visual_rect.get("height", 0.0)), 0.0)
	return visual_area > 0.0 and shape_area > visual_area * 1.75


func _world_2d_for_node(node: Node) -> World2D:
	if node is CanvasItem:
		return (node as CanvasItem).get_world_2d()
	if _scene_root is CanvasItem:
		return (_scene_root as CanvasItem).get_world_2d()
	if _root_window != null:
		return _root_window.world_2d
	return null


func _serialize_intersect_shape_results(results: Array, target_node: Node) -> Array:
	var hits: Array = []
	for result in results:
		if not result is Dictionary:
			continue
		var entry: Dictionary = result
		var collider = entry.get("collider", null)
		var collider_payload := {
			"path": "",
			"name": "",
			"type": type_string(typeof(collider)),
			"qa_id": "",
			"is_target": false,
		}
		if collider is Node:
			var collider_node := collider as Node
			collider_payload["path"] = str(collider_node.get_path())
			collider_payload["name"] = str(collider_node.name)
			collider_payload["type"] = collider_node.get_class()
			collider_payload["qa_id"] = str(collider_node.get_meta("qa_id", ""))
			collider_payload["is_target"] = collider_node == target_node
		hits.append({
			"collider": collider_payload,
			"collider_id": entry.get("collider_id", null),
			"shape": entry.get("shape", null),
			"rid": str(entry.get("rid", "")),
		})
	return hits


func _shape_from_target_report(target_node: Node, shape_report: Dictionary) -> Shape2D:
	if not target_node is CollisionObject2D:
		return null
	var collision_object := target_node as CollisionObject2D
	var owner_id := int(shape_report.get("owner_id", -1))
	var shape_index := int(shape_report.get("shape_index", -1))
	if owner_id < 0 or shape_index < 0:
		return null
	if shape_index >= collision_object.shape_owner_get_shape_count(owner_id):
		return null
	return collision_object.shape_owner_get_shape(owner_id, shape_index)


func _transform2d_from_payload(payload: Variant) -> Transform2D:
	if not payload is Dictionary:
		return Transform2D.IDENTITY
	var transform_payload: Dictionary = payload
	var x_axis := _vector2_from_payload(transform_payload.get("x", {}), Vector2.RIGHT)
	var y_axis := _vector2_from_payload(transform_payload.get("y", {}), Vector2.DOWN)
	var origin := _vector2_from_payload(transform_payload.get("origin", {}), Vector2.ZERO)
	return Transform2D(x_axis, y_axis, origin)


func _vector2_from_payload(payload: Variant, fallback: Vector2) -> Vector2:
	if payload is Dictionary:
		var dictionary: Dictionary = payload
		if _is_number(dictionary.get("x")) and _is_number(dictionary.get("y")):
			return Vector2(float(dictionary.get("x")), float(dictionary.get("y")))
	return fallback


func _distance_from_rect_to_point(rect_payload: Dictionary, point: Vector2) -> float:
	if rect_payload.is_empty():
		return INF
	var rect := Rect2(
		Vector2(float(rect_payload.get("x", 0.0)), float(rect_payload.get("y", 0.0))),
		Vector2(float(rect_payload.get("width", 0.0)), float(rect_payload.get("height", 0.0))),
	)
	if rect.has_point(point):
		return 0.0
	var clamped := Vector2(
		clampf(point.x, rect.position.x, rect.position.x + rect.size.x),
		clampf(point.y, rect.position.y, rect.position.y + rect.size.y),
	)
	return point.distance_to(clamped)


func _vector2_payload(value: Vector2) -> Dictionary:
	return {
		"x": value.x,
		"y": value.y,
		"length": value.length(),
	}


func _rect2_payload(rect: Rect2) -> Dictionary:
	return {
		"x": rect.position.x,
		"y": rect.position.y,
		"width": rect.size.x,
		"height": rect.size.y,
		"center": _vector2_payload(rect.get_center()),
	}


func _transform2d_payload(transform: Transform2D) -> Dictionary:
	return {
		"x": _vector2_payload(transform.x),
		"y": _vector2_payload(transform.y),
		"origin": _vector2_payload(transform.origin),
	}


func _is_allowed_physics_state_method(method: String) -> bool:
	if method == "" \
			or method.begins_with("qa_prepare_") \
			or method.begins_with("qa_apply_") \
			or method.begins_with("qa_mutate_") \
			or method.begins_with("qa_set_"):
		return false
	for character in method:
		var codepoint := character.unicode_at(0)
		if not ((codepoint >= 48 and codepoint <= 57) or (codepoint >= 65 and codepoint <= 90) or (codepoint >= 97 and codepoint <= 122) or character == "_"):
			return false
	if method == "qa_collision_probe":
		return true
	return (method.begins_with("get_qa_") and method.ends_with("_state")) \
		or (method.begins_with("get_") and method.ends_with("_qa_state")) \
		or (method.begins_with("qa_") and method.contains("diagnostic"))


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
			return _vector2_payload(value)
		TYPE_VECTOR2I:
			var vector2i: Vector2i = value
			return {"x": vector2i.x, "y": vector2i.y}
		TYPE_VECTOR3:
			var vector3: Vector3 = value
			return {"x": vector3.x, "y": vector3.y, "z": vector3.z, "length": vector3.length()}
		TYPE_RECT2:
			return _rect2_payload(value)
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
	var target := str(params.get("target", ""))
	var resolution := resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var target_node := resolution.get("node") as Node
	var position_result := _resolve_setup_position(params, "to")
	if not bool(position_result.get("ok", false)):
		return position_result
	var applied := _apply_global_position(
		target_node,
		position_result,
		bool(params.get("preserve_velocity", false)),
	)
	if not bool(applied.get("ok", false)):
		return applied
	var applied_data: Dictionary = applied.get("data", {})
	await _process_frames(1)
	return _ok(
		{
			"target": target,
			"resolved": str(resolution.get("resolved", "")),
			"position": applied_data,
		}
	)


func _fixture_teleport_to_target(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", ""))
	var destination := str(params.get("to", ""))
	var has_point := params.has("to_point")
	if target == "" or (destination == "" and not has_point and not params.has("position")):
		return _error(
			"invalid_params",
			"fixture.teleport_to_target requires non-empty target and destination",
			{
				"command": "fixture.teleport_to_target",
				"expected": {"target": "non-empty locator", "to": "locator, to_point provider, or position"},
				"actual": {"target": target, "to": destination, "to_point": params.get("to_point", null), "position": params.get("position", null)},
			},
		)
	return await _teleport_node(params)


func _spawn_scene(params: Dictionary) -> Dictionary:
	var scene_path := str(params.get("scene", ""))
	if scene_path == "" or not scene_path.begins_with("res://"):
		return _error(
			"invalid_params",
			"setup.spawn_scene requires a non-empty res:// scene path",
			{
				"command": "setup.spawn_scene",
				"expected": "res:// PackedScene path",
				"actual": scene_path,
			},
		)
	var scene_resource = load(scene_path)
	if scene_resource == null or not scene_resource is PackedScene:
		return _error(
			"scene_load_failed",
			"setup.spawn_scene could not load a PackedScene",
			{
				"scene": scene_path,
				"expected": "loadable PackedScene",
				"actual": "missing_or_invalid",
			},
		)

	var parent: Node = _scene_root
	var parent_target := str(params.get("parent", ""))
	var parent_resolved := stable_locator_for_node(parent)
	if parent_target != "":
		var parent_resolution := resolve_target(parent_target)
		if not bool(parent_resolution.get("ok", false)):
			return {"ok": false, "error": (parent_resolution.get("error", {}) as Dictionary).duplicate(true)}
		parent = parent_resolution.get("node") as Node
		parent_resolved = str(parent_resolution.get("resolved", ""))
	if parent == null:
		return _error(
			"target_not_found",
			"setup.spawn_scene parent is not available",
			{
				"parent": parent_target,
				"expected": "existing parent node",
				"actual": "missing",
			},
		)

	var instance := (scene_resource as PackedScene).instantiate()
	scene_resource = null
	if not instance is Node:
		return _error(
			"scene_instantiate_failed",
			"setup.spawn_scene did not instantiate a Node",
			{
				"scene": scene_path,
				"expected": "PackedScene root Node",
				"actual": type_string(typeof(instance)),
			},
		)
	var instance_node := instance as Node
	var requested_name := str(params.get("name", ""))
	if requested_name != "":
		instance_node.name = requested_name
	parent.add_child(instance_node)
	var qa_id := str(params.get("qa_id", ""))
	if qa_id != "":
		register_qa_id(instance_node, qa_id)

	var position_payload: Dictionary = {}
	if params.has("at") or params.has("position"):
		var position_result := _resolve_setup_position(params, "at")
		if not bool(position_result.get("ok", false)):
			instance_node.queue_free()
			return position_result
		var applied := _apply_global_position(instance_node, position_result)
		if not bool(applied.get("ok", false)):
			instance_node.queue_free()
			return applied
		position_payload = applied.get("data", {})
	await _process_frames(1)
	return _ok(
		{
			"scene": scene_path,
			"parent": parent_resolved,
			"resolved": "path:%s" % str(instance_node.get_path()),
			"target": stable_locator_for_node(instance_node),
			"position": position_payload,
		}
	)


func _set_property(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", ""))
	var property_name := str(params.get("property", ""))
	if property_name == "":
		return _error(
			"invalid_params",
			"setup.set_property requires a non-empty property name",
			{
				"command": "setup.set_property",
				"expected": "non-empty property",
				"actual": property_name,
			},
		)
	var resolution := resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var node := resolution.get("node") as Node
	if node == null or not _object_has_property(node, property_name):
		return _error(
			"property_not_found",
			"setup.set_property could not find the requested property",
			{
				"target": target,
				"property": property_name,
				"expected": "property on resolved node",
				"actual": node.get_class() if node != null else "missing",
			},
		)
	var before_value = _encode_setup_value(node.get(property_name))
	node.set(property_name, _decode_setup_value(params.get("value")))
	await _process_frames(1)
	return _ok(
		{
			"target": target,
			"resolved": str(resolution.get("resolved", "")),
			"property": property_name,
			"before": before_value,
			"after": _encode_setup_value(node.get(property_name)),
		}
	)


func _call_method(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", ""))
	var method_name := str(params.get("method", ""))
	if not method_name.begins_with("qa_"):
		return _error(
			"fixture_method_not_allowed",
			"setup.call_method only allows qa_ fixture methods",
			{
				"target": target,
				"method": method_name,
				"expected": "method name beginning with qa_",
				"actual": method_name,
			},
		)
	var resolution := resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var node := resolution.get("node") as Node
	if node == null or not node.has_method(method_name):
		return _error(
			"fixture_method_missing",
			"setup.call_method could not find the requested qa_ method",
			{
				"target": target,
				"method": method_name,
				"expected": "method on resolved node",
				"actual": node.get_class() if node != null else "missing",
			},
		)
	var args_value = params.get("args", [])
	var args: Array = args_value if args_value is Array else []
	var decoded_args: Array = []
	for arg in args:
		decoded_args.append(_decode_setup_value(arg))
	var result = node.callv(method_name, decoded_args)
	await _process_frames(1)
	return _ok(
		{
			"target": target,
			"resolved": str(resolution.get("resolved", "")),
			"method": method_name,
			"result": _encode_setup_value(result),
		}
	)


func _fixture_call(params: Dictionary) -> Dictionary:
	var driver_id := str(params.get("driver", "")).strip_edges()
	var action := str(params.get("action", "")).strip_edges()
	if driver_id == "" or action == "":
		return _error(
			"invalid_fixture_call",
			"fixture.call requires non-empty driver and action",
			{
				"command": "fixture.call",
				"expected": {"driver": "non-empty provider id", "action": "non-empty action"},
				"actual": params.duplicate(true),
			},
		)
	var call_params = params.get("params", {})
	if not (call_params is Dictionary):
		return _error(
			"invalid_fixture_call",
			"fixture.call params must be a dictionary when present",
			{
				"command": "fixture.call",
				"driver": driver_id,
				"action": action,
				"expected": "Dictionary",
				"actual": call_params,
			},
		)
	var driver := _provider_entry(_fixture_drivers, driver_id, "fixture_driver")
	if not bool(driver.get("ok", false)):
		return driver
	var entry: Dictionary = driver.get("entry", {})
	var owner: Object = entry.get("owner")
	var method_name := str(entry.get("method", ""))
	var request := {
		"driver": driver_id,
		"action": action,
		"params": (call_params as Dictionary).duplicate(true),
	}
	var result = owner.call(method_name, request)
	await _process_frames(1)
	if not (result is Dictionary):
		return _error(
			"invalid_fixture_result",
			"fixture driver must return a protocol dictionary",
			{
				"command": "fixture.call",
				"driver": driver_id,
				"action": action,
				"expected": {"ok": "boolean", "data": "Dictionary on success", "error": "Dictionary on failure"},
				"actual": _encode_setup_value(result),
			},
		)
	var result_dict: Dictionary = result
	if result_dict.has("ok") and not bool(result_dict.get("ok", false)):
		var error_payload = result_dict.get("error", {})
		if error_payload is Dictionary:
			return {"ok": false, "error": (error_payload as Dictionary).duplicate(true)}
		return _error(
			"fixture_driver_failed",
			"fixture driver reported failure",
			{"command": "fixture.call", "driver": driver_id, "action": action, "actual": result_dict.duplicate(true)},
		)
	if not bool(result_dict.get("ok", false)):
		return _error(
			"invalid_fixture_result",
			"fixture driver result must include ok: true on success",
			{"command": "fixture.call", "driver": driver_id, "action": action, "actual": result_dict.duplicate(true)},
		)
	var data = result_dict.get("data", {})
	if not (data is Dictionary):
		return _error(
			"invalid_fixture_result",
			"fixture driver success data must be a dictionary",
			{"command": "fixture.call", "driver": driver_id, "action": action, "actual": result_dict.duplicate(true)},
		)
	var payload: Dictionary = (data as Dictionary).duplicate(true)
	payload["driver"] = driver_id
	payload["action"] = action
	return _ok(payload)


# One drive_policy tick: call the registered (or builtin) policy with a
# read-only request, validate the returned desired-input list, then emit each
# input through the same joypad event paths as input.joy_motion /
# input.joy_button. Never mutates node state on the policy's behalf, and emits
# nothing when the returned shape is malformed. Frame pacing is owned by the
# scenario runner, so this command does not await frames.
func _policy_drive_tick(params: Dictionary) -> Dictionary:
	var policy_id := str(params.get("policy", "")).strip_edges()
	if policy_id == "":
		return _error(
			"invalid_policy_call",
			"policy.drive_tick requires a non-empty policy",
			{
				"command": "policy.drive_tick",
				"expected": {"policy": "non-empty policy provider id"},
				"actual": params.duplicate(true),
			},
		)
	var devices := _policy_device_allowlist(params.get("devices", null))
	if devices.is_empty():
		return _error(
			"invalid_policy_call",
			"policy.drive_tick requires a non-empty devices list of non-negative integers",
			{
				"command": "policy.drive_tick",
				"policy": policy_id,
				"expected": "non-empty list of non-negative integers",
				"actual": _encode_setup_value(params.get("devices", null)),
			},
		)
	var policy_params = params.get("params", {})
	if not (policy_params is Dictionary):
		return _error(
			"invalid_policy_call",
			"policy.drive_tick params must be a dictionary when present",
			{
				"command": "policy.drive_tick",
				"policy": policy_id,
				"expected": "Dictionary",
				"actual": _encode_setup_value(policy_params),
			},
		)
	var request := {
		"policy": policy_id,
		"tick": int(params.get("tick", 0)),
		"frame": int(params.get("frame", 0)),
		"devices": devices.duplicate(),
		"params": (policy_params as Dictionary).duplicate(true),
	}
	var result: Variant
	if policy_id.begins_with("builtin:"):
		var builtin := _call_builtin_policy(policy_id, request)
		if not bool(builtin.get("ok", false)):
			return builtin
		result = builtin.get("result", {})
	else:
		var provider := _provider_entry(_policy_providers, policy_id, "policy_provider")
		if not bool(provider.get("ok", false)):
			return provider
		var entry: Dictionary = provider.get("entry", {})
		var owner: Object = entry.get("owner")
		var method_name := str(entry.get("method", ""))
		result = owner.call(method_name, request)
	if not (result is Dictionary):
		return _error(
			"invalid_policy_result",
			"policy provider must return a dictionary with an 'inputs' list",
			{
				"command": "policy.drive_tick",
				"policy": policy_id,
				"expected": {"inputs": "list of {device, motion|button} objects"},
				"actual": _encode_setup_value(result),
			},
		)
	var raw_inputs = (result as Dictionary).get("inputs", null)
	if not (raw_inputs is Array):
		return _error(
			"invalid_policy_result",
			"policy result field 'inputs' must be a list",
			{
				"command": "policy.drive_tick",
				"policy": policy_id,
				"expected": "list of {device, motion|button} objects",
				"actual": _encode_setup_value(raw_inputs),
			},
		)
	# Validate the whole list before emitting anything so a malformed policy
	# result injects no input at all.
	var planned: Array[Dictionary] = []
	for index in range((raw_inputs as Array).size()):
		var validated := _validate_policy_input(policy_id, index, (raw_inputs as Array)[index], devices)
		if not bool(validated.get("ok", false)):
			return validated
		planned.append(validated.get("input", {}))
	var motions := 0
	var buttons := 0
	for input in planned:
		if input.has("axis_index"):
			_emit_joy_motion_event(int(input.get("device", 0)), int(input.get("axis_index", -1)), float(input.get("value", 0.0)))
			motions += 1
		else:
			_emit_joy_button_event(int(input.get("device", 0)), int(input.get("button_index", -1)), bool(input.get("pressed", false)))
			buttons += 1
	return _ok({
		"policy": policy_id,
		"tick": int(request.get("tick", 0)),
		"inputs_applied": planned.size(),
		"motions": motions,
		"buttons": buttons,
	})


# Zero touched joypad axes and release pressed joypad buttons for exactly the
# devices a drive_policy step drove, so a stopped policy leaves no input
# behind while inputs injected for other devices stay untouched.
func _policy_release_inputs(params: Dictionary) -> Dictionary:
	var devices := _policy_device_allowlist(params.get("devices", null))
	if devices.is_empty():
		return _error(
			"invalid_policy_call",
			"policy.release_inputs requires a non-empty devices list of non-negative integers",
			{
				"command": "policy.release_inputs",
				"expected": "non-empty list of non-negative integers",
				"actual": _encode_setup_value(params.get("devices", null)),
			},
		)
	var released_axes := 0
	for entry in _touched_joy_axes.values():
		if not entry is Dictionary:
			continue
		var device := int((entry as Dictionary).get("device", -1))
		if not devices.has(device):
			continue
		_emit_joy_motion_event(device, int((entry as Dictionary).get("axis_index", -1)), 0.0)
		released_axes += 1
	var released_buttons := 0
	for entry in _touched_joy_buttons.values():
		if not entry is Dictionary:
			continue
		var device := int((entry as Dictionary).get("device", -1))
		if not devices.has(device):
			continue
		_emit_joy_button_event(device, int((entry as Dictionary).get("button_index", -1)), false)
		released_buttons += 1
	if released_axes + released_buttons > 0:
		await _process_frames(1)
	return _ok({
		"released_axes": released_axes,
		"released_buttons": released_buttons,
	})


func _policy_device_allowlist(raw_devices: Variant) -> Array[int]:
	var devices: Array[int] = []
	if not (raw_devices is Array):
		return devices
	for entry in raw_devices:
		var device := -1
		if entry is int:
			device = int(entry)
		elif entry is float and is_equal_approx(float(entry), roundf(float(entry))):
			device = int(roundf(float(entry)))
		if device < 0:
			return []
		if not devices.has(device):
			devices.append(device)
	return devices


func _validate_policy_input(policy_id: String, index: int, raw_input: Variant, devices: Array[int]) -> Dictionary:
	# drive_policy results carry the policy id in every error and restrict inputs
	# to the step's `devices` allowlist; the shared entry validator handles both.
	return _validate_joy_input_entry(
		index, raw_input, devices, "invalid_policy_result", "policy.drive_tick", "policy input", {"policy": policy_id}
	)


# Validate one `{device, motion|button}` joypad input entry, the exact per-entry
# shape shared by policy.drive_tick results and the open-loop input.joy_state
# batch. `devices` is an allowlist when non-empty (drive_policy); an empty list
# allows any non-negative device (input.joy_state). `error_type`/`command`/`label`
# shape the failure envelope and messages, and `identity` (e.g. {"policy": id})
# is merged into every error's data. On success returns
# {"ok": true, "input": {device, axis|button name, axis_index|button_index, value|pressed}}.
func _validate_joy_input_entry(
	index: int,
	raw_input: Variant,
	devices: Array[int],
	error_type: String,
	command: String,
	label: String,
	identity: Dictionary,
) -> Dictionary:
	var device_expected := "integer from the step's devices list" if not devices.is_empty() else "non-negative integer"
	var expected := {
		"device": device_expected,
		"motion": "{axis, value} with a named axis and value from -1.0 to 1.0",
		"button": "{name, pressed} with a named button and boolean pressed",
	}
	if not (raw_input is Dictionary):
		return _joy_input_error(
			error_type, command, identity,
			"%s %d must be an object" % [label, index],
			expected, _encode_setup_value(raw_input),
		)
	var input: Dictionary = raw_input
	var raw_device = input.get("device", null)
	var device := -1
	if raw_device is int:
		device = int(raw_device)
	elif raw_device is float and is_equal_approx(float(raw_device), roundf(float(raw_device))):
		device = int(roundf(float(raw_device)))
	if device < 0:
		return _joy_input_error(
			error_type, command, identity,
			"%s %d field 'device' must be a non-negative integer" % [label, index],
			expected, _encode_setup_value(raw_device),
		)
	if not devices.is_empty() and not devices.has(device):
		return _joy_input_error(
			error_type, command, identity,
			"%s %d drives device %d outside the step's devices list" % [label, index, device],
			devices, device,
		)
	var has_motion := input.has("motion")
	var has_button := input.has("button")
	if has_motion == has_button:
		return _joy_input_error(
			error_type, command, identity,
			"%s %d must use exactly one of 'motion' or 'button'" % [label, index],
			expected, input.duplicate(true),
		)
	if has_motion:
		var motion = input.get("motion", null)
		if not (motion is Dictionary):
			return _joy_input_error(
				error_type, command, identity,
				"%s %d field 'motion' must be an object" % [label, index],
				expected, _encode_setup_value(motion),
			)
		var normalized_axis := _normalize_joy_axis(str((motion as Dictionary).get("axis", "")))
		if normalized_axis.is_empty():
			return _joy_input_error(
				error_type, command, identity,
				"%s %d motion 'axis' must be a supported named joypad axis" % [label, index],
				_supported_joy_axis_names(), _encode_setup_value((motion as Dictionary).get("axis", null)),
			)
		var raw_value = (motion as Dictionary).get("value", null)
		if not (raw_value is int or raw_value is float) or float(raw_value) < -1.0 or float(raw_value) > 1.0:
			return _joy_input_error(
				error_type, command, identity,
				"%s %d motion 'value' must be a number from -1.0 to 1.0" % [label, index],
				"number between -1.0 and 1.0", _encode_setup_value(raw_value),
			)
		return {"ok": true, "input": {
			"device": device,
			"axis": str(normalized_axis.get("name", "")),
			"axis_index": int(normalized_axis.get("axis_index", -1)),
			"value": float(raw_value),
		}}
	var button = input.get("button", null)
	if not (button is Dictionary):
		return _joy_input_error(
			error_type, command, identity,
			"%s %d field 'button' must be an object" % [label, index],
			expected, _encode_setup_value(button),
		)
	var normalized_button := _normalize_joy_button(str((button as Dictionary).get("name", "")))
	if normalized_button.is_empty():
		return _joy_input_error(
			error_type, command, identity,
			"%s %d button 'name' must be a supported named joypad button" % [label, index],
			_supported_joy_button_names(), _encode_setup_value((button as Dictionary).get("name", null)),
		)
	var raw_pressed = (button as Dictionary).get("pressed", null)
	if not (raw_pressed is bool):
		return _joy_input_error(
			error_type, command, identity,
			"%s %d button 'pressed' must be a boolean" % [label, index],
			"boolean", _encode_setup_value(raw_pressed),
		)
	return {"ok": true, "input": {
		"device": device,
		"button": str(normalized_button.get("name", "")),
		"button_index": int(normalized_button.get("button_index", -1)),
		"pressed": bool(raw_pressed),
	}}


# Build a joypad-entry validation error, merging provider identity (e.g.
# {"policy": id}) into the data envelope right after `command` so the policy
# path keeps its historic {command, policy, expected, actual} field order.
func _joy_input_error(error_type: String, command: String, identity: Dictionary, message: String, expected: Variant, actual: Variant) -> Dictionary:
	var data := {"command": command}
	for key in identity:
		data[key] = identity[key]
	data["expected"] = expected
	data["actual"] = actual
	return _error(error_type, message, data)


func _call_builtin_policy(policy_id: String, request: Dictionary) -> Dictionary:
	match policy_id:
		"builtin:steer_toward":
			return _builtin_policy_steer_toward(request)
	return _error(
		"unknown_builtin_policy",
		"policy.drive_tick does not provide this builtin policy",
		{
			"command": "policy.drive_tick",
			"policy": policy_id,
			"expected": ["builtin:steer_toward"],
		},
	)


# Minimal builtin policy for smoke tests and simple games: each tick it reads
# the steered node and target positions and returns normalized stick motion
# toward the target for the first driven device, or zeroed motion once the
# steered node is within tolerance.
func _builtin_policy_steer_toward(request: Dictionary) -> Dictionary:
	var params: Dictionary = request.get("params", {})
	var node_locator := str(params.get("node", "")).strip_edges()
	var target_locator := str(params.get("target", "")).strip_edges()
	if node_locator == "" or target_locator == "":
		return _error(
			"invalid_policy_call",
			"builtin:steer_toward requires non-empty 'node' and 'target' locator params",
			{
				"command": "policy.drive_tick",
				"policy": "builtin:steer_toward",
				"expected": {"node": "steered node locator", "target": "destination locator"},
				"actual": params.duplicate(true),
			},
		)
	var stick := str(params.get("stick", "left")).strip_edges().to_lower()
	if stick != "left" and stick != "right":
		return _error(
			"invalid_policy_call",
			"builtin:steer_toward param 'stick' must be 'left' or 'right'",
			{
				"command": "policy.drive_tick",
				"policy": "builtin:steer_toward",
				"expected": ["left", "right"],
				"actual": _encode_setup_value(params.get("stick", null)),
			},
		)
	var tolerance := 8.0
	if params.has("tolerance"):
		var raw_tolerance = params.get("tolerance", null)
		if not (raw_tolerance is int or raw_tolerance is float) or float(raw_tolerance) <= 0.0:
			return _error(
				"invalid_policy_call",
				"builtin:steer_toward param 'tolerance' must be a positive number",
				{
					"command": "policy.drive_tick",
					"policy": "builtin:steer_toward",
					"expected": "positive number",
					"actual": _encode_setup_value(raw_tolerance),
				},
			)
		tolerance = float(raw_tolerance)
	var node_result := resolve_target(node_locator)
	if not bool(node_result.get("ok", false)):
		return node_result
	var target_result := resolve_target(target_locator)
	if not bool(target_result.get("ok", false)):
		return target_result
	var node = node_result.get("node", null)
	var target = target_result.get("node", null)
	if not (node is Node2D) or not (target is Node2D):
		return _error(
			"invalid_policy_call",
			"builtin:steer_toward requires Node2D 'node' and 'target' nodes",
			{
				"command": "policy.drive_tick",
				"policy": "builtin:steer_toward",
				"expected": "Node2D locators",
				"actual": {
					"node": (node as Object).get_class() if node is Object else "missing",
					"target": (target as Object).get_class() if target is Object else "missing",
				},
			},
		)
	var devices: Array = request.get("devices", [])
	var device := int(devices[0]) if not devices.is_empty() else 0
	var delta: Vector2 = (target as Node2D).global_position - (node as Node2D).global_position
	var direction := Vector2.ZERO
	if delta.length() > tolerance:
		direction = delta.normalized()
	return {"ok": true, "result": {"inputs": [
		{"device": device, "motion": {"axis": "%s_x" % stick, "value": clampf(direction.x, -1.0, 1.0)}},
		{"device": device, "motion": {"axis": "%s_y" % stick, "value": clampf(direction.y, -1.0, 1.0)}},
	]}}


func _resolve_setup_position(params: Dictionary, locator_field: String) -> Dictionary:
	var has_locator := params.has(locator_field)
	var has_position := params.has("position")
	var has_point := params.has("to_point")
	var destination_count := (1 if has_locator else 0) + (1 if has_position else 0) + (1 if has_point else 0)
	if destination_count > 1:
		return _error(
			"invalid_params",
			"setup position requires exactly one destination source",
			{
				"expected": "only one of %s, position, or to_point" % locator_field,
				"actual": {
					locator_field: params.get(locator_field, null),
					"position": params.get("position", null),
					"to_point": params.get("to_point", null),
				},
			},
		)
	if has_point:
		return _resolve_setup_point_provider(params.get("to_point"), params.get("offset", null))
	if params.has(locator_field):
		var target := str(params.get(locator_field, ""))
		var resolution := resolve_target(target)
		if not bool(resolution.get("ok", false)):
			return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
		var node := resolution.get("node") as Node
		var position := _global_position_for_node(node)
		if position.is_empty():
			return _error(
				"target_position_unavailable",
				"Resolved target does not expose a supported global position",
				{
					"target": target,
					"resolved": str(resolution.get("resolved", "")),
					"expected": "Node2D, Control, or Node3D target",
					"actual": node.get_class() if node != null else "missing",
				},
			)
		position["source"] = target
		position["source_resolved"] = str(resolution.get("resolved", ""))
		return _ok(_position_with_offset(position, params.get("offset", null)))
	if params.has("position"):
		var position_value = params.get("position")
		var position := _coerce_position_dictionary(position_value)
		if position.is_empty():
			return _error(
				"invalid_params",
				"setup position requires numeric x and y fields",
				{
					"expected": {"x": "number", "y": "number", "z": "optional number"},
					"actual": position_value,
				},
			)
		return _ok(_position_with_offset(position, params.get("offset", null)))
	return _error(
		"invalid_params",
		"setup position requires a locator or explicit position",
		{
			"expected": "%s locator, position object, or to_point provider" % locator_field,
			"actual": params.duplicate(true),
		},
	)


func _resolve_setup_point_provider(raw_point_request: Variant, raw_offset: Variant = null) -> Dictionary:
	if not raw_point_request is Dictionary:
		return _error(
			"invalid_params",
			"to_point requires a provider request object",
			{
				"expected": {"provider": "qa:<id>", "method": "qa_point_probe", "id": "logical-point-id"},
				"actual": raw_point_request,
			},
		)
	var point_request: Dictionary = raw_point_request
	var provider_target := str(point_request.get("provider", "")).strip_edges()
	var method_name := str(point_request.get("method", "qa_point_probe")).strip_edges()
	var point_id := str(point_request.get("id", point_request.get("point_id", ""))).strip_edges()
	if provider_target == "" or method_name == "" or point_id == "":
		return _error(
			"invalid_params",
			"to_point requires non-empty provider, method, and id",
			{
				"expected": {"provider": "non-empty locator", "method": "qa_ method", "id": "non-empty logical id"},
				"actual": point_request.duplicate(true),
			},
		)
	var method_check := _validate_readonly_qa_probe_method(method_name)
	if not bool(method_check.get("ok", false)):
		return _error(
			"point_provider_not_allowed",
			"to_point provider method must be a read-only QA probe method",
			{
				"provider": provider_target,
				"method": method_name,
				"expected": "qa_ method without mutating prefix",
				"actual": method_check.get("reason", "invalid_method"),
			},
		)
	var provider_resolution := resolve_target(provider_target)
	if not bool(provider_resolution.get("ok", false)):
		return {"ok": false, "error": (provider_resolution.get("error", {}) as Dictionary).duplicate(true)}
	var provider_node := provider_resolution.get("node") as Node
	if provider_node == null or not provider_node.has_method(method_name):
		return _error(
			"point_provider_missing",
			"to_point provider does not expose the requested method",
			{
				"provider": provider_target,
				"resolved": str(provider_resolution.get("resolved", "")),
				"method": method_name,
				"expected": "method on resolved provider",
				"actual": provider_node.get_class() if provider_node != null else "missing",
			},
		)
	var result = provider_node.call(method_name, point_id)
	if not result is Dictionary:
		return _error(
			"invalid_point_provider_result",
			"to_point provider must return a Dictionary",
			{
				"provider": provider_target,
				"method": method_name,
				"id": point_id,
				"expected": "Dictionary",
				"actual": type_string(typeof(result)),
			},
		)
	var point_result: Dictionary = result
	if not bool(point_result.get("ok", false)):
		return _error(
			"point_provider_rejected",
			"to_point provider could not resolve the logical point",
			{
				"provider": provider_target,
				"method": method_name,
				"id": point_id,
				"expected": "ok point result",
				"actual": point_result.duplicate(true),
			},
		)
	var coordinate_space := str(point_result.get("coordinate_space", "world_2d")).strip_edges()
	if coordinate_space != "world_2d":
		return _error(
			"unsupported_coordinate_space",
			"to_point provider must return world_2d coordinates",
			{
				"provider": provider_target,
				"method": method_name,
				"id": point_id,
				"expected": "world_2d",
				"actual": coordinate_space,
			},
		)
	var raw_point = point_result.get("point", point_result.get("position", point_result.get("global_position", null)))
	var position := _coerce_position_dictionary(raw_point)
	if position.is_empty():
		return _error(
			"invalid_point_provider_result",
			"to_point provider result must include numeric point coordinates",
			{
				"provider": provider_target,
				"method": method_name,
				"id": point_id,
				"expected": {"point": {"x": "number", "y": "number"}},
				"actual": point_result.duplicate(true),
			},
		)
	position["source"] = "to_point:%s" % point_id
	position["source_resolved"] = str(provider_resolution.get("resolved", ""))
	position["provider"] = provider_target
	position["provider_method"] = method_name
	position["logical_id"] = str(point_result.get("logical_id", point_id))
	return _ok(_position_with_offset(position, raw_offset))


func _validate_readonly_qa_probe_method(method_name: String) -> Dictionary:
	if not method_name.begins_with("qa_"):
		return {"ok": false, "reason": "method_must_begin_with_qa_"}
	var mutating_prefixes := ["qa_set_", "qa_add_", "qa_remove_", "qa_delete_", "qa_clear_", "qa_queue_", "qa_free", "qa_mutate_"]
	for prefix in mutating_prefixes:
		if method_name.begins_with(prefix):
			return {"ok": false, "reason": "mutating_method_prefix"}
	return {"ok": true}


func _position_with_offset(position: Dictionary, raw_offset: Variant) -> Dictionary:
	var result := position.duplicate(true)
	if raw_offset == null:
		return result
	var offset := _coerce_position_dictionary(raw_offset)
	if offset.is_empty():
		return result
	result["x"] = float(result.get("x", 0.0)) + float(offset.get("x", 0.0))
	result["y"] = float(result.get("y", 0.0)) + float(offset.get("y", 0.0))
	if result.has("z") or offset.has("z"):
		result["z"] = float(result.get("z", 0.0)) + float(offset.get("z", 0.0))
	result["offset"] = offset
	return result


func _apply_global_position(node: Node, position_result: Dictionary, preserve_velocity := false) -> Dictionary:
	if node == null:
		return _error(
			"target_not_found",
			"No node matched the requested target",
			{
				"expected": "existing runtime node",
				"actual": "missing",
			},
		)
	var position_data: Dictionary = position_result.get("data", {})
	var x := float(position_data.get("x", 0.0))
	var y := float(position_data.get("y", 0.0))
	if node is Node2D:
		(node as Node2D).global_position = Vector2(x, y)
		return _ok({"x": x, "y": y, "velocity_reset": _reset_transient_physics_state(node, preserve_velocity)})
	if node is Control:
		(node as Control).global_position = Vector2(x, y)
		return _ok({"x": x, "y": y, "velocity_reset": false})
	if node is Node3D:
		var current := (node as Node3D).global_position
		var z := float(position_data.get("z", current.z))
		(node as Node3D).global_position = Vector3(x, y, z)
		return _ok({"x": x, "y": y, "z": z, "velocity_reset": _reset_transient_physics_state(node, preserve_velocity)})
	return _error(
		"target_position_unsupported",
		"Resolved target does not support global-position setup",
		{
			"target": describe_node(node),
			"expected": "Node2D, Control, or Node3D target",
			"actual": node.get_class(),
		},
	)


func _reset_transient_physics_state(node: Node, preserve_velocity: bool) -> bool:
	if preserve_velocity:
		return false
	if node is CharacterBody2D:
		(node as CharacterBody2D).velocity = Vector2.ZERO
		return true
	if node is CharacterBody3D:
		(node as CharacterBody3D).velocity = Vector3.ZERO
		return true
	if node is RigidBody2D:
		(node as RigidBody2D).linear_velocity = Vector2.ZERO
		(node as RigidBody2D).angular_velocity = 0.0
		return true
	if node is RigidBody3D:
		(node as RigidBody3D).linear_velocity = Vector3.ZERO
		(node as RigidBody3D).angular_velocity = Vector3.ZERO
		return true
	return false


func _global_position_for_node(node: Node) -> Dictionary:
	if node is Node2D:
		var position_2d := (node as Node2D).global_position
		return {"x": position_2d.x, "y": position_2d.y}
	if node is Control:
		var position_control := (node as Control).global_position
		return {"x": position_control.x, "y": position_control.y}
	if node is Node3D:
		var position_3d := (node as Node3D).global_position
		return {"x": position_3d.x, "y": position_3d.y, "z": position_3d.z}
	return {}


func _coerce_position_dictionary(value: Variant) -> Dictionary:
	if not value is Dictionary:
		return {}
	var position: Dictionary = value
	if not _is_number(position.get("x")) or not _is_number(position.get("y")):
		return {}
	var result := {
		"x": float(position.get("x")),
		"y": float(position.get("y")),
	}
	if position.has("z"):
		if not _is_number(position.get("z")):
			return {}
		result["z"] = float(position.get("z"))
	return result


func _object_has_property(object: Object, property_name: String) -> bool:
	for property in object.get_property_list():
		if not property is Dictionary:
			continue
		if str((property as Dictionary).get("name", "")) == property_name:
			return true
	return false


func _decode_setup_value(value: Variant) -> Variant:
	if value is Dictionary:
		var dictionary: Dictionary = value
		var value_type := str(dictionary.get("type", ""))
		match value_type:
			"Vector2":
				if _is_number(dictionary.get("x")) and _is_number(dictionary.get("y")):
					return Vector2(float(dictionary.get("x")), float(dictionary.get("y")))
			"Vector2i":
				if _is_number(dictionary.get("x")) and _is_number(dictionary.get("y")):
					return Vector2i(int(dictionary.get("x")), int(dictionary.get("y")))
			"Vector3":
				if _is_number(dictionary.get("x")) and _is_number(dictionary.get("y")) and _is_number(dictionary.get("z")):
					return Vector3(float(dictionary.get("x")), float(dictionary.get("y")), float(dictionary.get("z")))
			"Color":
				if _is_number(dictionary.get("r")) and _is_number(dictionary.get("g")) and _is_number(dictionary.get("b")):
					return Color(
						float(dictionary.get("r")),
						float(dictionary.get("g")),
						float(dictionary.get("b")),
						float(dictionary.get("a", 1.0))
					)
		var decoded := {}
		for key in dictionary.keys():
			decoded[key] = _decode_setup_value(dictionary.get(key))
		return decoded
	if value is Array:
		var decoded_array: Array = []
		for item in value:
			decoded_array.append(_decode_setup_value(item))
		return decoded_array
	return value


func _encode_setup_value(value: Variant) -> Variant:
	match typeof(value):
		TYPE_NIL, TYPE_BOOL, TYPE_INT, TYPE_FLOAT, TYPE_STRING:
			return value
		TYPE_VECTOR2:
			var vector2: Vector2 = value
			return {"type": "Vector2", "x": vector2.x, "y": vector2.y}
		TYPE_VECTOR2I:
			var vector2i: Vector2i = value
			return {"type": "Vector2i", "x": vector2i.x, "y": vector2i.y}
		TYPE_VECTOR3:
			var vector3: Vector3 = value
			return {"type": "Vector3", "x": vector3.x, "y": vector3.y, "z": vector3.z}
		TYPE_COLOR:
			var color: Color = value
			return {"type": "Color", "r": color.r, "g": color.g, "b": color.b, "a": color.a}
		TYPE_ARRAY:
			var encoded_array: Array = []
			for item in value:
				encoded_array.append(_encode_setup_value(item))
			return encoded_array
		TYPE_DICTIONARY:
			var encoded_dictionary := {}
			for key in value.keys():
				encoded_dictionary[str(key)] = _encode_setup_value(value.get(key))
			return encoded_dictionary
	if value is Node:
		return stable_locator_for_node(value as Node)
	return str(value)


func _is_number(value: Variant) -> bool:
	return typeof(value) == TYPE_INT or typeof(value) == TYPE_FLOAT


func _mouse_click(raw_x: Variant, raw_y: Variant) -> Dictionary:
	var x := _coerce_coordinate(raw_x)
	var y := _coerce_coordinate(raw_y)
	if x == null or y == null:
		return _error(
			"invalid_params",
			"input.mouse_click requires integer viewport coordinates",
			{
				"command": "input.mouse_click",
				"expected": "integer --x and --y viewport coordinates",
				"actual": {
					"x": raw_x,
					"y": raw_y,
				},
			},
		)
	var position := Vector2(float(x), float(y))
	var visible_rect := viewport_rect()
	if visible_rect.size.x <= 0.0 or visible_rect.size.y <= 0.0 or not visible_rect.has_point(position):
		return _error(
			"invalid_params",
			"input.mouse_click requires coordinates inside the visible viewport",
			{
				"command": "input.mouse_click",
				"expected": _rect_to_array(visible_rect),
				"actual": {
					"x": x,
					"y": y,
				},
			},
		)

	var hovered := _hovered_control_at_position(position)
	await _emit_left_click(position)
	var data := {
		"device": "mouse",
		"button": "left",
		"position": {
			"x": x,
			"y": y,
		},
	}
	if hovered != null:
		data["target"] = stable_locator_for_node(hovered)
		data["resolved"] = "path:%s" % str(hovered.get_path())
	return _ok(data)


func _mouse_wheel(params: Dictionary) -> Dictionary:
	var raw_x: Variant = params.get("x", null)
	var raw_y: Variant = params.get("y", null)
	var x := _coerce_number_coordinate(raw_x)
	var y := _coerce_number_coordinate(raw_y)
	if x == null or y == null:
		return _error(
			"invalid_params",
			"input.mouse_wheel requires numeric viewport coordinates",
			{
				"command": "input.mouse_wheel",
				"expected": "numeric --x and --y viewport coordinates",
				"actual": {
					"x": raw_x,
					"y": raw_y,
				},
			},
		)
	var dy := float(params.get("dy", 0.0))
	var dx := float(params.get("dx", 0.0))
	if is_zero_approx(dy) and is_zero_approx(dx):
		return _error(
			"invalid_params",
			"input.mouse_wheel requires a non-zero --dy or --dx delta",
			{
				"command": "input.mouse_wheel",
				"expected": "non-zero scroll delta on --dy or --dx",
				"actual": {
					"dy": dy,
					"dx": dx,
				},
			},
		)
	var position := Vector2(float(x), float(y))
	var visible_rect := viewport_rect()
	if visible_rect.size.x <= 0.0 or visible_rect.size.y <= 0.0 or not visible_rect.has_point(position):
		return _error(
			"invalid_params",
			"input.mouse_wheel requires coordinates inside the visible viewport",
			{
				"command": "input.mouse_wheel",
				"expected": _rect_to_array(visible_rect),
				"actual": {
					"x": x,
					"y": y,
				},
			},
		)

	var buttons: Array[String] = []
	if not is_zero_approx(dy):
		var vertical_down := dy > 0.0
		var vertical_button := MOUSE_BUTTON_WHEEL_DOWN if vertical_down else MOUSE_BUTTON_WHEEL_UP
		var vertical_name := "wheel_down" if vertical_down else "wheel_up"
		var vertical_pairs := int(max(1, ceil(absf(dy))))
		var vertical_factor := absf(dy) / float(vertical_pairs)
		for _i in range(vertical_pairs):
			_emit_mouse_wheel_event(position, vertical_button, vertical_factor, true)
			_emit_mouse_wheel_event(position, vertical_button, vertical_factor, false)
			await _process_frames(1)
			buttons.append(vertical_name)
	if not is_zero_approx(dx):
		var horizontal_right := dx > 0.0
		var horizontal_button := MOUSE_BUTTON_WHEEL_RIGHT if horizontal_right else MOUSE_BUTTON_WHEEL_LEFT
		var horizontal_name := "wheel_right" if horizontal_right else "wheel_left"
		var horizontal_pairs := int(max(1, ceil(absf(dx))))
		var horizontal_factor := absf(dx) / float(horizontal_pairs)
		for _i in range(horizontal_pairs):
			_emit_mouse_wheel_event(position, horizontal_button, horizontal_factor, true)
			_emit_mouse_wheel_event(position, horizontal_button, horizontal_factor, false)
			await _process_frames(1)
			buttons.append(horizontal_name)

	var data := {
		"device": "mouse",
		"position": {
			"x": x,
			"y": y,
		},
		"dx": dx,
		"dy": dy,
		"buttons": buttons,
	}
	return _ok(data)


func _touch_down(params: Dictionary) -> Dictionary:
	var raw_x: Variant = params.get("x", null)
	var raw_y: Variant = params.get("y", null)
	var x := _coerce_number_coordinate(raw_x)
	var y := _coerce_number_coordinate(raw_y)
	if x == null or y == null:
		return _error(
			"invalid_params",
			"input.touch_down requires numeric viewport coordinates",
			{
				"command": "input.touch_down",
				"expected": "numeric --x and --y viewport coordinates",
				"actual": {
					"x": raw_x,
					"y": raw_y,
				},
			},
		)
	var position := Vector2(float(x), float(y))
	var visible_rect := viewport_rect()
	if visible_rect.size.x <= 0.0 or visible_rect.size.y <= 0.0 or not visible_rect.has_point(position):
		return _error(
			"invalid_params",
			"input.touch_down requires coordinates inside the visible viewport",
			{
				"command": "input.touch_down",
				"expected": _rect_to_array(visible_rect),
				"actual": {
					"x": x,
					"y": y,
				},
			},
		)
	var index := _non_negative_int_param(params, "index", 0, "input.touch_down")
	if not bool(index.get("ok", false)):
		return index
	var finger := int(index.get("value", 0))
	if _touched_screen_points.has(finger):
		return _error(
			"touch_index_held",
			"input.touch_down index %d is already held" % finger,
			{
				"command": "input.touch_down",
				"index": finger,
			},
		)

	_note_touch_emulation()
	_emit_screen_touch_event(finger, position, true)
	await _process_frames(1)
	return _ok({
		"device": "touch",
		"index": finger,
		"position": {
			"x": x,
			"y": y,
		},
		"pressed": true,
	})


func _touch_move(params: Dictionary) -> Dictionary:
	var raw_x: Variant = params.get("x", null)
	var raw_y: Variant = params.get("y", null)
	var x := _coerce_number_coordinate(raw_x)
	var y := _coerce_number_coordinate(raw_y)
	if x == null or y == null:
		return _error(
			"invalid_params",
			"input.touch_move requires numeric viewport coordinates",
			{
				"command": "input.touch_move",
				"expected": "numeric --x and --y viewport coordinates",
				"actual": {
					"x": raw_x,
					"y": raw_y,
				},
			},
		)
	var position := Vector2(float(x), float(y))
	var visible_rect := viewport_rect()
	if visible_rect.size.x <= 0.0 or visible_rect.size.y <= 0.0 or not visible_rect.has_point(position):
		return _error(
			"invalid_params",
			"input.touch_move requires coordinates inside the visible viewport",
			{
				"command": "input.touch_move",
				"expected": _rect_to_array(visible_rect),
				"actual": {
					"x": x,
					"y": y,
				},
			},
		)
	var index := _non_negative_int_param(params, "index", 0, "input.touch_move")
	if not bool(index.get("ok", false)):
		return index
	var frames_param := _non_negative_int_param(params, "frames", 1, "input.touch_move")
	if not bool(frames_param.get("ok", false)):
		return frames_param
	var frames := int(frames_param.get("value", 1))
	if frames > MAX_INPUT_GESTURE_FRAMES:
		return _error(
			"invalid_params",
			"input.touch_move frames must not exceed %d" % MAX_INPUT_GESTURE_FRAMES,
			{
				"command": "input.touch_move",
				"field": "frames",
				"expected": "frames <= %d" % MAX_INPUT_GESTURE_FRAMES,
				"actual": frames,
			},
		)
	var finger := int(index.get("value", 0))
	if not _touched_screen_points.has(finger):
		return _error(
			"touch_index_not_held",
			"input.touch_move index %d is not held" % finger,
			{
				"command": "input.touch_move",
				"index": finger,
			},
		)

	_note_touch_emulation()
	var last_position: Vector2 = (_touched_screen_points[finger] as Dictionary).get("position", position)
	var steps := maxi(frames, 1)
	var prev_position := last_position
	var final_relative := Vector2.ZERO
	for i in range(1, steps + 1):
		var target_position := last_position.lerp(position, float(i) / float(steps))
		if i == steps:
			target_position = position
		var relative := target_position - prev_position
		var velocity := relative / _effective_frame_delta()
		_emit_screen_drag_event(finger, target_position, relative, velocity)
		final_relative = relative
		prev_position = target_position
		await _process_frames(1)
	return _ok({
		"device": "touch",
		"index": finger,
		"position": {
			"x": x,
			"y": y,
		},
		"relative": {
			"x": final_relative.x,
			"y": final_relative.y,
		},
		"frames": frames,
	})


func _touch_up(params: Dictionary) -> Dictionary:
	var index := _non_negative_int_param(params, "index", 0, "input.touch_up")
	if not bool(index.get("ok", false)):
		return index
	var finger := int(index.get("value", 0))
	if not _touched_screen_points.has(finger):
		return _error(
			"touch_index_not_held",
			"input.touch_up index %d is not held" % finger,
			{
				"command": "input.touch_up",
				"index": finger,
			},
		)

	var position: Vector2 = (_touched_screen_points[finger] as Dictionary).get("position", Vector2.ZERO)
	var raw_x: Variant = params.get("x", null)
	var raw_y: Variant = params.get("y", null)
	if raw_x != null or raw_y != null:
		var x := _coerce_number_coordinate(raw_x)
		var y := _coerce_number_coordinate(raw_y)
		if x == null or y == null:
			return _error(
				"invalid_params",
				"input.touch_up requires numeric viewport coordinates",
				{
					"command": "input.touch_up",
					"expected": "numeric --x and --y viewport coordinates",
					"actual": {
						"x": raw_x,
						"y": raw_y,
					},
				},
			)
		position = Vector2(float(x), float(y))
		var visible_rect := viewport_rect()
		if visible_rect.size.x <= 0.0 or visible_rect.size.y <= 0.0 or not visible_rect.has_point(position):
			return _error(
				"invalid_params",
				"input.touch_up requires coordinates inside the visible viewport",
				{
					"command": "input.touch_up",
					"expected": _rect_to_array(visible_rect),
					"actual": {
						"x": x,
						"y": y,
					},
				},
			)

	_note_touch_emulation()
	_emit_screen_touch_event(finger, position, false)
	await _process_frames(1)
	return _ok({
		"device": "touch",
		"index": finger,
		"position": {
			"x": position.x,
			"y": position.y,
		},
		"pressed": false,
	})


func _touch_tap(params: Dictionary) -> Dictionary:
	var raw_x: Variant = params.get("x", null)
	var raw_y: Variant = params.get("y", null)
	var x := _coerce_number_coordinate(raw_x)
	var y := _coerce_number_coordinate(raw_y)
	if x == null or y == null:
		return _error(
			"invalid_params",
			"input.touch_tap requires numeric viewport coordinates",
			{
				"command": "input.touch_tap",
				"expected": "numeric --x and --y viewport coordinates",
				"actual": {
					"x": raw_x,
					"y": raw_y,
				},
			},
		)
	var position := Vector2(float(x), float(y))
	var visible_rect := viewport_rect()
	if visible_rect.size.x <= 0.0 or visible_rect.size.y <= 0.0 or not visible_rect.has_point(position):
		return _error(
			"invalid_params",
			"input.touch_tap requires coordinates inside the visible viewport",
			{
				"command": "input.touch_tap",
				"expected": _rect_to_array(visible_rect),
				"actual": {
					"x": x,
					"y": y,
				},
			},
		)
	var index := _non_negative_int_param(params, "index", 0, "input.touch_tap")
	if not bool(index.get("ok", false)):
		return index
	var finger := int(index.get("value", 0))
	if _touched_screen_points.has(finger):
		return _error(
			"touch_index_held",
			"input.touch_tap index %d is already held" % finger,
			{
				"command": "input.touch_tap",
				"index": finger,
			},
		)

	_note_touch_emulation()
	var hovered := _hovered_control_at_position(position)
	_emit_screen_touch_event(finger, position, true)
	await _process_frames(1)
	_emit_screen_touch_event(finger, position, false)
	await _process_frames(1)
	var data := {
		"device": "touch",
		"index": finger,
		"position": {
			"x": x,
			"y": y,
		},
	}
	if hovered != null:
		data["target"] = stable_locator_for_node(hovered)
		data["resolved"] = "path:%s" % str(hovered.get_path())
	return _ok(data)


func _drag(params: Dictionary) -> Dictionary:
	var raw_from_x: Variant = params.get("from_x", null)
	var raw_from_y: Variant = params.get("from_y", null)
	var raw_to_x: Variant = params.get("to_x", null)
	var raw_to_y: Variant = params.get("to_y", null)
	var from_x := _coerce_number_coordinate(raw_from_x)
	var from_y := _coerce_number_coordinate(raw_from_y)
	var to_x := _coerce_number_coordinate(raw_to_x)
	var to_y := _coerce_number_coordinate(raw_to_y)
	if from_x == null or from_y == null or to_x == null or to_y == null:
		return _error(
			"invalid_params",
			"input.drag requires numeric viewport coordinates",
			{
				"command": "input.drag",
				"expected": "numeric --from-x/--from-y/--to-x/--to-y viewport coordinates",
				"actual": {
					"from_x": raw_from_x,
					"from_y": raw_from_y,
					"to_x": raw_to_x,
					"to_y": raw_to_y,
				},
			},
		)
	var from_position := Vector2(float(from_x), float(from_y))
	var to_position := Vector2(float(to_x), float(to_y))
	var visible_rect := viewport_rect()
	if visible_rect.size.x <= 0.0 or visible_rect.size.y <= 0.0 or not visible_rect.has_point(from_position):
		return _error(
			"invalid_params",
			"input.drag requires coordinates inside the visible viewport",
			{
				"command": "input.drag",
				"expected": _rect_to_array(visible_rect),
				"actual": {
					"endpoint": "from",
					"x": from_x,
					"y": from_y,
				},
			},
		)
	if not visible_rect.has_point(to_position):
		return _error(
			"invalid_params",
			"input.drag requires coordinates inside the visible viewport",
			{
				"command": "input.drag",
				"expected": _rect_to_array(visible_rect),
				"actual": {
					"endpoint": "to",
					"x": to_x,
					"y": to_y,
				},
			},
		)
	var frames_param := _non_negative_int_param(params, "frames", 8, "input.drag")
	if not bool(frames_param.get("ok", false)):
		return frames_param
	var frames := int(frames_param.get("value", 8))
	if frames < 1:
		return _error(
			"invalid_params",
			"input.drag frames must be at least 1",
			{
				"command": "input.drag",
				"field": "frames",
				"expected": "frames >= 1",
				"actual": frames,
			},
		)
	if frames > MAX_INPUT_GESTURE_FRAMES:
		return _error(
			"invalid_params",
			"input.drag frames must not exceed %d" % MAX_INPUT_GESTURE_FRAMES,
			{
				"command": "input.drag",
				"field": "frames",
				"expected": "frames <= %d" % MAX_INPUT_GESTURE_FRAMES,
				"actual": frames,
			},
		)
	var index_param := _non_negative_int_param(params, "index", 0, "input.drag")
	if not bool(index_param.get("ok", false)):
		return index_param
	var finger := int(index_param.get("value", 0))
	var pointer := str(params.get("pointer", "touch"))
	if pointer != "touch" and pointer != "mouse":
		return _error(
			"invalid_params",
			"input.drag pointer must be touch or mouse",
			{
				"command": "input.drag",
				"field": "pointer",
				"expected": ["touch", "mouse"],
				"actual": pointer,
			},
		)
	var steps := maxi(frames, 1)

	if pointer == "touch":
		if _touched_screen_points.has(finger):
			return _error(
				"touch_index_held",
				"input.drag index %d is already held" % finger,
				{
					"command": "input.drag",
					"index": finger,
				},
			)
		_note_touch_emulation()
		_emit_screen_touch_event(finger, from_position, true)
		await _process_frames(1)
		var prev_position := from_position
		for i in range(1, steps + 1):
			var target_position := from_position.lerp(to_position, float(i) / float(steps))
			if i == steps:
				target_position = to_position
			var relative := target_position - prev_position
			var velocity := relative / _effective_frame_delta()
			_emit_screen_drag_event(finger, target_position, relative, velocity)
			prev_position = target_position
			await _process_frames(1)
		_emit_screen_touch_event(finger, to_position, false)
		await _process_frames(1)
		return _ok({
			"device": "touch",
			"pointer": pointer,
			"index": finger,
			"from": {
				"x": from_x,
				"y": from_y,
			},
			"to": {
				"x": to_x,
				"y": to_y,
			},
			"frames": frames,
			"steps": steps,
		})

	# pointer == "mouse": drive Control drag detection with a left-button-masked
	# motion; the plain _emit_mouse_motion_event carries no mask.
	_emit_mouse_motion_event(from_position)
	await _process_frames(1)
	_emit_mouse_button_event(from_position, true)
	await _process_frames(1)
	var mouse_prev := from_position
	for j in range(1, steps + 1):
		var mouse_target := from_position.lerp(to_position, float(j) / float(steps))
		if j == steps:
			mouse_target = to_position
		var mouse_relative := mouse_target - mouse_prev
		var mouse_velocity := mouse_relative / _effective_frame_delta()
		_emit_mouse_drag_motion_event(mouse_target, mouse_relative, mouse_velocity)
		mouse_prev = mouse_target
		await _process_frames(1)
	_emit_mouse_button_event(to_position, false)
	await _process_frames(1)
	return _ok({
		"device": "mouse",
		"pointer": pointer,
		"from": {
			"x": from_x,
			"y": from_y,
		},
		"to": {
			"x": to_x,
			"y": to_y,
		},
		"frames": frames,
		"steps": steps,
	})


func _press_key(raw_key: String) -> Dictionary:
	var normalized := _normalize_key(raw_key)
	if normalized.is_empty():
		return _error(
			"invalid_params",
			"input.key requires a supported named key",
			{
				"command": "input.key",
				"expected": _supported_key_names(),
				"actual": raw_key,
			},
		)

	var key_name := str(normalized.get("name", ""))
	var keycode := int(normalized.get("keycode", 0))
	var unicode := int(normalized.get("unicode", 0))
	await _tap_key(keycode, unicode)
	var data := {
		"device": "keyboard",
		"key": key_name,
		"unicode": unicode,
	}
	_append_focus_owner(data)
	return _ok(data)


func _type_text(params: Dictionary) -> Dictionary:
	var raw_text = params.get("text", null)
	if not (raw_text is String) or (raw_text as String).is_empty():
		return _error(
			"invalid_params",
			"input.text requires a non-empty text string",
			{
				"command": "input.text",
				"field": "text",
				"expected": "non-empty string",
				"actual": raw_text,
			},
		)

	var text := raw_text as String
	var max_text_length := MAX_INPUT_GESTURE_FRAMES / 2
	if text.length() > max_text_length:
		return _error(
			"invalid_params",
			"input.text text is too long for one gesture",
			{
				"command": "input.text",
				"field": "text",
				"expected": "text length <= %d" % max_text_length,
				"actual": text.length(),
			},
		)

	for i in text.length():
		var codepoint := text.unicode_at(i)
		var keycode := OS.find_keycode_from_string(text.substr(i, 1).to_upper())
		await _tap_key(keycode, codepoint)

	var data := {
		"device": "keyboard",
		"text": text,
		"characters": text.length(),
	}
	_append_focus_owner(data)
	return _ok(data)


func _press_joy_button(params: Dictionary) -> Dictionary:
	var raw_button := str(params.get("button", ""))
	var normalized := _normalize_joy_button(raw_button)
	if normalized.is_empty():
		return _error(
			"invalid_params",
			"input.joy_button requires a supported named joypad button",
			{
				"command": "input.joy_button",
				"expected": _supported_joy_button_names(),
				"actual": raw_button,
			},
		)

	var device := _non_negative_int_param(params, "device", 0, "input.joy_button")
	if not bool(device.get("ok", false)):
		return device
	var frames := _non_negative_int_param(params, "frames", 1, "input.joy_button")
	if not bool(frames.get("ok", false)):
		return frames

	var button_name := str(normalized.get("name", ""))
	var button_index := int(normalized.get("button_index", -1))
	_emit_joy_button_event(int(device.get("value", 0)), button_index, true)
	await _process_frames(int(frames.get("value", 1)))
	_emit_joy_button_event(int(device.get("value", 0)), button_index, false)
	await _process_frames(1)
	var data := {
		"device": "joypad",
		"device_id": int(device.get("value", 0)),
		"button": button_name,
		"button_index": button_index,
		"frames": int(frames.get("value", 1)),
	}
	_append_focus_owner(data)
	return _ok(data)


func _joy_motion(params: Dictionary) -> Dictionary:
	var raw_axis := str(params.get("axis", ""))
	var normalized := _normalize_joy_axis(raw_axis)
	if normalized.is_empty():
		return _error(
			"invalid_params",
			"input.joy_motion requires a supported named joypad axis",
			{
				"command": "input.joy_motion",
				"expected": _supported_joy_axis_names(),
				"actual": raw_axis,
			},
		)

	var raw_value = params.get("value", null)
	if not (raw_value is int or raw_value is float):
		return _error(
			"invalid_params",
			"input.joy_motion value must be a number from -1.0 to 1.0",
			{
				"command": "input.joy_motion",
				"field": "value",
				"expected": "number between -1.0 and 1.0",
				"actual": raw_value,
			},
		)
	var value := float(raw_value)
	if value < -1.0 or value > 1.0:
		return _error(
			"invalid_params",
			"input.joy_motion value must be between -1.0 and 1.0",
			{
				"command": "input.joy_motion",
				"field": "value",
				"expected": "number between -1.0 and 1.0",
				"actual": value,
			},
		)

	var device := _non_negative_int_param(params, "device", 0, "input.joy_motion")
	if not bool(device.get("ok", false)):
		return device
	var frames := _non_negative_int_param(params, "frames", 1, "input.joy_motion")
	if not bool(frames.get("ok", false)):
		return frames

	var axis_name := str(normalized.get("name", ""))
	var axis_index := int(normalized.get("axis_index", -1))
	_emit_joy_motion_event(int(device.get("value", 0)), axis_index, value)
	await _process_frames(int(frames.get("value", 1)))
	var data := {
		"device": "joypad",
		"device_id": int(device.get("value", 0)),
		"axis": axis_name,
		"axis_index": axis_index,
		"value": value,
		"frames": int(frames.get("value", 1)),
	}
	_append_focus_owner(data)
	return _ok(data)


# Open-loop batch joypad input: apply many devices' inputs in the SAME frame,
# including button holds and axis positions that persist across steps. The whole
# `inputs` list is validated before anything is emitted (a malformed entry fails
# the step and injects nothing); then every entry is emitted back-to-back with no
# frame awaits between them, so concurrent cross-device holds land on one frame.
# Buttons stay pressed and axes stay latched until a later step changes them;
# existing _touched_joy_* tracking plus finalize/input.reset release them (this
# command adds no cleanup of its own). `frames` (default 1) is the post-emit wait
# (0 = emit and continue same-frame). Unlike input.joy_button there is no device
# allowlist: any non-negative device is accepted.
func _input_joy_state(params: Dictionary) -> Dictionary:
	var raw_inputs = params.get("inputs", null)
	if not (raw_inputs is Array) or (raw_inputs as Array).is_empty():
		return _error(
			"invalid_params",
			"input.joy_state requires a non-empty 'inputs' list",
			{
				"command": "input.joy_state",
				"field": "inputs",
				"expected": "non-empty list of {device, motion|button} objects",
				"actual": _encode_setup_value(raw_inputs),
			},
		)
	var frames := _non_negative_int_param(params, "frames", 1, "input.joy_state")
	if not bool(frames.get("ok", false)):
		return frames
	# Validate the WHOLE list before emitting anything so a malformed entry
	# injects no input at all. No device allowlist: any non-negative device.
	var no_allowlist: Array[int] = []
	var planned: Array[Dictionary] = []
	for index in range((raw_inputs as Array).size()):
		var validated := _validate_joy_input_entry(
			index, (raw_inputs as Array)[index], no_allowlist, "invalid_params", "input.joy_state", "input", {}
		)
		if not bool(validated.get("ok", false)):
			return validated
		planned.append(validated.get("input", {}))
	# Emit every entry back-to-back with no frame await between them so all
	# devices' inputs are applied in a single frame, mirroring the drive_policy
	# tick emit loop.
	for input in planned:
		if input.has("axis_index"):
			_emit_joy_motion_event(int(input.get("device", 0)), int(input.get("axis_index", -1)), float(input.get("value", 0.0)))
		else:
			_emit_joy_button_event(int(input.get("device", 0)), int(input.get("button_index", -1)), bool(input.get("pressed", false)))
	await _process_frames(int(frames.get("value", 1)))
	var data := {
		"device": "joypad",
		"inputs_applied": planned.size(),
		"inputs": planned,
		"frames": int(frames.get("value", 1)),
	}
	_append_focus_owner(data)
	return _ok(data)


# Announce one or more virtual joypads as CONNECTED by firing the engine's
# Input.joy_connection_changed signal — the SAME signal the platform layer emits
# on a real controller connect — so a local-multiplayer lobby that listens for it
# ("press A to join", spawn-a-player) runs its connect handler under headless QA,
# where no physical pad ever fires it. `devices` is a required non-empty list of
# non-negative ints (deduped/normalized via the shared _policy_device_allowlist
# helper); `name` is optional QA metadata (see the engine-limitation note below).
# Connect is idempotent: a device already faked is NOT re-emitted (so a listener
# cannot double-join) but the request still succeeds. Every faked device is
# tracked in the static _faked_joypads set so finalize/input.reset can announce a
# matching disconnect for exactly what was faked and joy_disconnect's real-pad
# guard knows what the QA layer owns. One delivery frame is awaited after the
# whole batch so listeners settle.
#
# ENGINE LIMITATION (Godot 4.6.2): `Input.joy_connection_changed` is a SIGNAL, not
# a bound method. The roster-mutating C++ entry point
# `Input::joy_connection_changed(device, connected, name, guid)` is not exposed to
# GDScript (`Input.has_method("joy_connection_changed")` == false), and the signal
# carries only `(device, connected)`. So this fires the signal but does NOT add
# the device to `Input.get_connected_joypads()`, and `name` is not delivered to
# the engine (`get_joy_name` is unaffected). Enumeration-based lobbies that poll
# `get_connected_joypads()` therefore cannot be faked on this engine — only
# signal-driven connect/disconnect handling can. `name` is kept as accepted,
# echoed QA metadata (and to keep the step contract stable) for forward
# compatibility if a future engine binds the four-argument method.
func _joy_connect(params: Dictionary) -> Dictionary:
	var devices := _validate_joy_setup_devices(params, "setup.joy_connect")
	if not bool(devices.get("ok", false)):
		return devices
	var device_list: Array[int] = devices.get("devices", [] as Array[int])
	for device in device_list:
		if device in Input.get_connected_joypads():
			return _error(
				"invalid_params",
				"setup.joy_connect cannot claim a real connected joypad",
				{"command": "setup.joy_connect", "device": device, "connected_joypads": _connected_joypads_array()},
			)
	var name := "QA Virtual Pad"
	if params.has("name"):
		var raw_name = params.get("name", null)
		if not (raw_name is String) or (raw_name as String).strip_edges() == "":
			return _error(
				"invalid_params",
				"setup.joy_connect 'name' must be a non-empty string",
				{
					"command": "setup.joy_connect",
					"field": "name",
					"expected": "non-empty string",
					"actual": _encode_setup_value(raw_name),
				},
			)
		name = (raw_name as String).strip_edges()
	for device in device_list:
		# Idempotent: only fire `connected` for a device not already announced, so
		# a listener cannot double-join; refresh the tracked name either way.
		var already_faked := _faked_joypads.has(device)
		_faked_joypads[device] = {"device": device, "name": name}
		if not already_faked:
			Input.joy_connection_changed.emit(device, true)
	await _process_frames(1)
	return _ok({
		"devices": device_list,
		"name": name,
		"announced_joypads": _announced_joypads_array(),
		"connected_joypads": _connected_joypads_array(),
	})


# Announce one or more QA-faked joypads as DISCONNECTED by firing the same
# Input.joy_connection_changed signal with connected == false, so a scenario can
# exercise mid-match controller-drop handling (pause-on-drop, the classic
# couch-game bug class). GUARD: only devices previously faked via setup.joy_connect
# may be disconnected. Firing a disconnect for a device the QA layer never
# announced could correspond to a real physical pad and wrongly pause the game or
# drop a real player, so a request naming any non-faked device fails with
# invalid_params and emits nothing. Disconnected devices are removed from
# _faked_joypads tracking. One delivery frame is awaited after the whole batch.
# See _joy_connect for the Godot 4.6.2 engine limitation (signal-only; the
# get_connected_joypads() roster is unaffected).
func _joy_disconnect(params: Dictionary) -> Dictionary:
	var devices := _validate_joy_setup_devices(params, "setup.joy_disconnect")
	if not bool(devices.get("ok", false)):
		return devices
	var device_list: Array[int] = devices.get("devices", [] as Array[int])
	var not_faked: Array[int] = []
	for device in device_list:
		if not _faked_joypads.has(device):
			not_faked.append(device)
	if not not_faked.is_empty():
		return _error(
			"invalid_params",
			"setup.joy_disconnect can only disconnect joypads faked via setup.joy_connect",
			{
				"command": "setup.joy_disconnect",
				"field": "devices",
				"expected": "devices previously faked via setup.joy_connect",
				"actual": not_faked,
				"faked_devices": _faked_joypads.keys(),
			},
		)
	for device in device_list:
		if device in Input.get_connected_joypads():
			return _error(
				"invalid_params",
				"setup.joy_disconnect cannot disconnect a real connected joypad",
				{"command": "setup.joy_disconnect", "device": device, "connected_joypads": _connected_joypads_array()},
			)
	await _release_injected_joypad_devices(device_list)
	for device in device_list:
		_faked_joypads.erase(device)
		Input.joy_connection_changed.emit(device, false)
	await _process_frames(1)
	return _ok({
		"devices": device_list,
		"announced_joypads": _announced_joypads_array(),
		"connected_joypads": _connected_joypads_array(),
	})


# Validate the shared `devices` field for setup.joy_connect/disconnect: a required
# non-empty list of non-negative ints. Reuses _policy_device_allowlist for the
# dedup/normalize pass (drops duplicates, coerces whole floats). Returns
# {"ok": true, "devices": Array[int]} or an invalid_params error dictionary.
func _validate_joy_setup_devices(params: Dictionary, command: String) -> Dictionary:
	var raw_devices = params.get("devices", null)
	if not (raw_devices is Array) or (raw_devices as Array).is_empty():
		return _error(
			"invalid_params",
			"%s requires a non-empty 'devices' list" % command,
			{
				"command": command,
				"field": "devices",
				"expected": "non-empty list of non-negative integers",
				"actual": _encode_setup_value(raw_devices),
			},
		)
	var devices := _policy_device_allowlist(raw_devices)
	if devices.is_empty():
		return _error(
			"invalid_params",
			"%s 'devices' entries must be non-negative integers" % command,
			{
				"command": command,
				"field": "devices",
				"expected": "non-negative integers",
				"actual": _encode_setup_value(raw_devices),
			},
		)
	return {"ok": true, "devices": devices}


# Sorted device indices the QA layer has announced-as-connected and not yet
# disconnected (the static _faked_joypads tracking set). This is the observable
# contract the connect/disconnect success payloads echo, since on Godot 4.6.2 the
# real Input.get_connected_joypads() roster cannot be faked (see _joy_connect).
func _announced_joypads_array() -> Array[int]:
	var announced: Array[int] = []
	for device in _faked_joypads.keys():
		announced.append(int(device))
	announced.sort()
	return announced


# The real, engine-truth Input.get_connected_joypads() copied into a plain
# JSON-compatible Array[int]. Echoed for transparency; on Godot 4.6.2 it does NOT
# reflect QA-announced pads (signal-only, no roster mutation), so
# `announced_joypads` is the field scenarios should assert against. Games that
# poll the roster should read effective_connected_joypads() instead.
func _connected_joypads_array() -> Array[int]:
	var connected: Array[int] = []
	for device in Input.get_connected_joypads():
		connected.append(int(device))
	return connected


func _emit_action_event(action: String, pressed: bool) -> void:
	var event := InputEventAction.new()
	event.action = action
	event.pressed = pressed
	Input.parse_input_event(event)


func _emit_mouse_button_event(position: Vector2, pressed: bool) -> void:
	var event := InputEventMouseButton.new()
	event.button_index = MOUSE_BUTTON_LEFT
	event.pressed = pressed
	event.position = position
	event.global_position = position
	event.button_mask = MOUSE_BUTTON_MASK_LEFT if pressed else 0
	Input.parse_input_event(event)


func _emit_mouse_motion_event(position: Vector2) -> void:
	var event := InputEventMouseMotion.new()
	event.position = position
	event.global_position = position
	Input.parse_input_event(event)


# Mouse-drag motion carrying the left-button mask so Control drag detection fires;
# the plain _emit_mouse_motion_event sets no mask. Used by the input.drag mouse path.
func _emit_mouse_drag_motion_event(position: Vector2, relative: Vector2, velocity: Vector2) -> void:
	var event := InputEventMouseMotion.new()
	event.position = position
	event.global_position = position
	event.relative = relative
	event.velocity = velocity
	event.button_mask = MOUSE_BUTTON_MASK_LEFT
	Input.parse_input_event(event)


func _emit_mouse_wheel_event(position: Vector2, button_index: int, factor: float, pressed: bool) -> void:
	var event := InputEventMouseButton.new()
	event.button_index = button_index
	event.pressed = pressed
	event.factor = factor
	event.position = position
	event.global_position = position
	event.button_mask = 0
	Input.parse_input_event(event)


func _emit_screen_touch_event(index: int, position: Vector2, pressed: bool) -> void:
	var event := InputEventScreenTouch.new()
	event.index = index
	event.position = position
	event.pressed = pressed
	Input.parse_input_event(event)
	if pressed:
		_touched_screen_points[index] = {"position": position}
	else:
		_touched_screen_points.erase(index)


func _emit_screen_drag_event(index: int, position: Vector2, relative: Vector2, velocity: Vector2) -> void:
	var event := InputEventScreenDrag.new()
	event.index = index
	event.position = position
	event.relative = relative
	event.velocity = velocity
	Input.parse_input_event(event)
	_touched_screen_points[index] = {"position": position}


# Frame delta used to derive InputEventScreenDrag.velocity. Uses the live root
# window's process delta when available (deterministic under --fixed-fps), else a
# 1/60 fallback so velocity is finite even before the first frame is measured.
func _effective_frame_delta() -> float:
	if _root_window != null:
		var delta := _root_window.get_process_delta_time()
		if delta > 0.0:
			return delta
	return 1.0 / 60.0


# One-time informational note: while emulate_mouse_from_touch (Godot default on)
# is enabled, injected touch also synthesizes mouse events. Never mutates the
# setting; logs once per process through the addon's existing print channel.
func _note_touch_emulation() -> void:
	if _touch_emulation_note_emitted:
		return
	_touch_emulation_note_emitted = true
	if bool(ProjectSettings.get_setting("input_devices/pointing/emulate_mouse_from_touch", true)):
		print("[godot-qa] injected touch also synthesizes mouse events (input_devices/pointing/emulate_mouse_from_touch is enabled)")


func _tap_key(keycode: int, unicode: int = 0) -> void:
	_emit_key_event(keycode, true, unicode)
	await _process_frames(1)
	_emit_key_event(keycode, false, unicode)
	await _process_frames(1)


func _emit_key_event(keycode: int, pressed: bool, unicode: int = 0) -> void:
	var event := InputEventKey.new()
	event.keycode = keycode
	event.physical_keycode = keycode
	event.unicode = unicode
	event.pressed = pressed
	event.echo = false
	Input.parse_input_event(event)


func _emit_joy_button_event(device: int, button_index: int, pressed: bool) -> void:
	var event := InputEventJoypadButton.new()
	event.device = device
	event.button_index = button_index
	event.pressed = pressed
	event.pressure = 1.0 if pressed else 0.0
	Input.parse_input_event(event)
	var key := "%d:%d" % [device, button_index]
	if pressed:
		_touched_joy_buttons[key] = {"device": device, "button_index": button_index}
	else:
		_touched_joy_buttons.erase(key)


func _emit_joy_motion_event(device: int, axis_index: int, value: float) -> void:
	var event := InputEventJoypadMotion.new()
	event.device = device
	event.axis = axis_index
	event.axis_value = value
	Input.parse_input_event(event)
	var key := "%d:%d" % [device, axis_index]
	if is_zero_approx(value):
		_touched_joy_axes.erase(key)
	else:
		_touched_joy_axes[key] = {"device": device, "axis_index": axis_index}


func _process_frames(count: int) -> void:
	for _i in range(max(count, 0)):
		await _root_window.get_tree().process_frame


func _emit_left_click(position: Vector2) -> void:
	_emit_mouse_motion_event(position)
	await _process_frames(1)
	_emit_mouse_button_event(position, true)
	await _process_frames(1)
	_emit_mouse_button_event(position, false)
	await _process_frames(1)


func _hovered_control_at_position(position: Vector2) -> Control:
	var visible_controls: Array[Control] = QaSnapshot.visible_controls(_scene_root)
	var best_control: Control = null
	var best_area := INF
	var best_depth := -1
	for control in visible_controls:
		if control == null:
			continue
		var rect := control.get_global_rect()
		if not rect.has_point(position):
			continue
		var area := rect.size.x * rect.size.y
		var depth := str(control.get_path()).count("/")
		if best_control == null or area < best_area or (is_equal_approx(area, best_area) and depth > best_depth):
			best_control = control
			best_area = area
			best_depth = depth
	if best_control != null:
		return best_control
	if _root_window != null and _root_window.has_method("gui_get_hovered_control"):
		var hovered = _root_window.gui_get_hovered_control()
		if hovered is Control:
			return hovered as Control
	return best_control


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


func _normalize_key(raw_key: String) -> Dictionary:
	var key_name := raw_key.strip_edges().to_lower()
	if key_name == "":
		return {}
	return (SUPPORTED_KEYS.get(key_name, {}) as Dictionary).duplicate(true)


func _normalize_joy_button(raw_button: String) -> Dictionary:
	var button_name := raw_button.strip_edges().to_lower()
	if button_name == "":
		return {}
	return (SUPPORTED_JOY_BUTTONS.get(button_name, {}) as Dictionary).duplicate(true)


func _supported_key_names() -> Array[String]:
	var canonical_names: Array[String] = []
	var seen := {}
	for entry in SUPPORTED_KEYS.values():
		if not entry is Dictionary:
			continue
		var name := str((entry as Dictionary).get("name", ""))
		if name == "" or seen.has(name):
			continue
		seen[name] = true
		canonical_names.append(name)
	canonical_names.sort()
	return canonical_names


func _supported_joy_button_names() -> Array[String]:
	var canonical_names: Array[String] = []
	var seen := {}
	for entry in SUPPORTED_JOY_BUTTONS.values():
		if not entry is Dictionary:
			continue
		var name := str((entry as Dictionary).get("name", ""))
		if name == "" or seen.has(name):
			continue
		seen[name] = true
		canonical_names.append(name)
	canonical_names.sort()
	return canonical_names


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


func _normalize_joy_axis(raw_axis: String) -> Dictionary:
	var axis_name := raw_axis.strip_edges().to_lower()
	if axis_name == "":
		return {}
	return (SUPPORTED_JOY_AXES.get(axis_name, {}) as Dictionary).duplicate(true)


func _supported_joy_axis_names() -> Array[String]:
	var canonical_names: Array[String] = []
	var seen := {}
	for entry in SUPPORTED_JOY_AXES.values():
		if not entry is Dictionary:
			continue
		var name := str((entry as Dictionary).get("name", ""))
		if name == "" or seen.has(name):
			continue
		seen[name] = true
		canonical_names.append(name)
	canonical_names.sort()
	return canonical_names


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
	return _global_position_for_node(node)


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
	if field == "text" and node is Control and _object_has_property(node, "text"):
		return str(node.get("text"))
	if field == "position":
		return _global_position_for_node(node)
	if field == "velocity":
		return velocity_for_node(node)
	if field == "rotation":
		return rotation_for_node(node)
	if _object_has_property(node, field):
		return node.get(field)
	return null


func capture_temporal_sample(target: String, field: String) -> Dictionary:
	var resolution := resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"target": target, "field": field, "error": (resolution.get("error", {}) as Dictionary).duplicate(true), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec()}
	var node := resolution.get("node") as Node
	return {"target": target, "resolved": str(resolution.get("resolved", "")), "field": field, "value": read_node_field(node, field), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec()}


func sample_temporal_window(target: String, field: String, window: Dictionary, sample_every_frames: int) -> Array:
	var samples: Array = []
	var cadence := max(sample_every_frames, 1)
	var sampling: Dictionary = window.get("temporal_sampling", {}) if window.get("temporal_sampling", {}) is Dictionary else {}
	if str(window.get("mode", "frames")) == "seconds":
		var seconds := max(float(window.get("value", 0.0)), 0.0)
		var start := Time.get_ticks_msec()
		while (Time.get_ticks_msec() - start) / 1000.0 <= seconds:
			var sample := capture_temporal_sample(target, field)
			sample["cadence"] = {"sample_every_frames": cadence}
			if not sampling.is_empty():
				sample["temporal_sampling"] = sampling.duplicate(true)
				sample["sampling_mode"] = str(sampling.get("mode", ""))
			samples.append(sample)
			await _process_frames(cadence)
	else:
		var frame_count := max(int(window.get("value", 0)), 1)
		for _i in range(frame_count):
			var sample := capture_temporal_sample(target, field)
			sample["cadence"] = {"sample_every_frames": cadence}
			if not sampling.is_empty():
				sample["temporal_sampling"] = sampling.duplicate(true)
				sample["sampling_mode"] = str(sampling.get("mode", ""))
			samples.append(sample)
			await _process_frames(cadence)
	return samples


func write_temporal_evidence(kind: String, target: String, payload: Dictionary) -> Dictionary:
	if payload.has("temporal_sampling") and payload.get("temporal_sampling", {}) is Dictionary:
		payload["sampling_mode"] = str((payload.get("temporal_sampling", {}) as Dictionary).get("mode", ""))
	if not payload.has("cadence") and payload.has("window") and payload.get("window", {}) is Dictionary:
		var window: Dictionary = payload.get("window", {})
		if window.has("cadence"):
			payload["cadence"] = (window.get("cadence", {}) as Dictionary).duplicate(true)
	var temporal_dir := str(payload.get("temporal_dir", ""))
	if temporal_dir == "":
		temporal_dir = "user://qa_artifacts/temporal"
	DirAccess.make_dir_recursive_absolute(temporal_dir)
	var safe_target := target.replace(":", "_").replace("/", "_")
	var file_path := "%s/%s_%s.json" % [temporal_dir, kind, safe_target]
	var file := FileAccess.open(file_path, FileAccess.WRITE)
	if file != null:
		file.store_string(JSON.stringify(payload, "	"))
		file.close()
	return {"temporal_evidence": file_path, "temporal_dir": temporal_dir}
