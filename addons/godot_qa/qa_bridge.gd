extends Node

const QaArgs = preload("res://addons/godot_qa/qa_args.gd")
const QaLiveSessionHost = preload("res://addons/godot_qa/qa_live_session_host.gd")
const QaProtocol = preload("res://addons/godot_qa/qa_protocol.gd")
const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")
const QaLiveEdit = preload("res://addons/godot_qa/qa_live_edit.gd")
const QaRecorder = preload("res://addons/godot_qa/qa_recorder.gd")
const QaSnapshot = preload("res://addons/godot_qa/qa_snapshot.gd")
const QaProfiler = preload("res://addons/godot_qa/qa_profiler.gd")
const QaSoak = preload("res://addons/godot_qa/qa_soak.gd")
const QaScenarioExecutor = preload("res://addons/godot_qa/qa_scenario_executor.gd")

const PROFILE_MONITORS := {
	"TIME_FPS": Performance.TIME_FPS,
	"TIME_PROCESS": Performance.TIME_PROCESS,
	"TIME_PHYSICS_PROCESS": Performance.TIME_PHYSICS_PROCESS,
	"OBJECT_COUNT": Performance.OBJECT_COUNT,
	"OBJECT_NODE_COUNT": Performance.OBJECT_NODE_COUNT,
	"RENDER_TOTAL_OBJECTS_IN_FRAME": Performance.RENDER_TOTAL_OBJECTS_IN_FRAME,
	"RENDER_TOTAL_PRIMITIVES_IN_FRAME": Performance.RENDER_TOTAL_PRIMITIVES_IN_FRAME,
	"RENDER_TOTAL_DRAW_CALLS_IN_FRAME": Performance.RENDER_TOTAL_DRAW_CALLS_IN_FRAME,
	"MEMORY_STATIC": Performance.MEMORY_STATIC,
	"MEMORY_STATIC_MAX": Performance.MEMORY_STATIC_MAX,
}
const MAX_TIME_SCALE := 64.0
# Overlapping live commands wait their turn in a bounded FIFO queue instead of
# being rejected outright. Only a saturated queue or an over-long wait falls
# back to the compatible busy error (see _bridge_busy_error).
const BRIDGE_QUEUE_MAX_WAIT_MSEC := 30000
const BRIDGE_QUEUE_MAX_DEPTH := 32
const ANDROID_BOOTSTRAP_VERSION := 1
const ANDROID_BOOTSTRAP_PATH := "user://godot_qa/android_live_bootstrap.json"
const ANDROID_ARTIFACTS_PREFIX := "user://godot_qa/live"
# Marker lines printed on stderr around every warm `scenario.run`, so the CLI can
# cut this scenario's slice out of the long-lived session log. stderr is the same
# unbuffered stream engine `ERROR:`/`WARNING:` lines take, so a line between the
# two markers really did happen during this scenario. Must stay byte-identical to
# WARM_LOG_MARKER_PREFIX in crates/godot-qa-scenario/src/run/warm.rs.
const WARM_LOG_MARKER_PREFIX := "[godot-qa] scenario-log"

var enabled := false
var activation_source := "disabled"
var runtime_options := {}
# Per-scenario QA user args (the `--qa-*` args after `--`). In a warm session
# these change per scenario; the game reads them via QaArgs.effective_user_args()
# instead of OS.get_cmdline_user_args() so it re-seeds RNG / re-reads fixtures.
# Empty in cold/idle, where effective_user_args() falls back to the cmdline.
var _scenario_user_args: PackedStringArray = PackedStringArray()
var _scenario_user_args_override := false
var _held_actions := {}
var _runtime_errors: Array = []
var _live_session := {}
var _live_edit: RefCounted = null
var _recorder: Node = null
var _session_logs: Array[Dictionary] = []
var _stop_requested := false
var _soak: RefCounted = null
var _bridge_command_in_progress := false
var _bridge_command_diagnostics := {}
# FIFO queue of monotonically increasing tickets. The ticket at the head is the
# one currently serving (or about to serve); every other ticket awaits its turn.
var _bridge_queue: Array[int] = []
var _bridge_queue_next_ticket := 0
var _android_live_host: Node = null
var _android_session_token := ""
var _android_pending_session := {}
const RECORDED_INPUT_COMMANDS := {
	"input.tap_action": true,
	"input.hold_action": true,
	"input.release_action": true,
	"input.click_target": true,
	"input.key": true,
	"input.text": true,
	"input.mouse_click": true,
	"input.joy_button": true,
	"input.joy_motion": true,
	"input.joy_state": true,
}

func _ready() -> void:
	var parsed_args := QaArgs.parse_user_args(OS.get_cmdline_user_args())
	runtime_options = parsed_args.get("options", {})

	if parsed_args.get("enabled", false):
		enabled = true
		activation_source = "user_args"
	elif bool(ProjectSettings.get_setting(QaArgs.ENABLE_SETTING, false)):
		enabled = true
		activation_source = "project_settings"
	else:
		enabled = false
		activation_source = "disabled"

	if enabled:
		print("[godot-qa] runtime bridge enabled")
	if OS.has_feature("android") and OS.has_feature("template") and OS.is_debug_build():
		_prepare_android_live_session()

func get_status() -> Dictionary:
	return {
		"enabled": enabled,
		"runtime_qa_active": is_runtime_qa_active(),
		"activation_source": activation_source,
		"display_server": DisplayServer.get_name(),
		"options": runtime_options,
	}


func is_runtime_qa_active() -> bool:
	return enabled


func get_runtime_errors() -> Array:
	return _runtime_errors


func clear_runtime_errors() -> void:
	_runtime_errors.clear()


# Per-scenario QA user args accessor used by QaArgs.effective_user_args(). Games
# that branch on project_args (seed, fixtures, test_mode) call this through the
# shim so warm sessions deliver the CURRENT scenario's args, not the session
# launch args. Returns empty in cold/idle, where the shim uses the cmdline.
func current_scenario_user_args() -> PackedStringArray:
	return _scenario_user_args


func has_scenario_user_args_override() -> bool:
	return _scenario_user_args_override


# Push a scenario's QA options to the game before its scene is instantiated.
# Sets the per-scenario user args (the primary, pull-based contract) and, if a
# node implements the optional qa_apply_options(options) hook, pushes the parsed
# options too. Returns whether a push hook was found. Called by the shared
# executor for every scenario (cold and warm); in cold the args equal the
# cmdline, so behaviour is unchanged.
func apply_scenario_options(scenario: Dictionary) -> bool:
	# Only a warm/live session needs per-scenario args pushed: there the cmdline
	# carries the SESSION launch args, not the scenario's. On the cold one-shot
	# (_live_session empty) the cmdline already IS this scenario's args, so we
	# leave _scenario_user_args empty and let effective_user_args() fall back to
	# the cmdline — keeping cold byte-for-byte unchanged.
	if _live_session.is_empty():
		return false
	var project_args: Array = scenario.get("project_args", []) if scenario.get("project_args", []) is Array else []
	var user_args := PackedStringArray()
	for arg in project_args:
		user_args.append(str(arg))
	_scenario_user_args = user_args
	_scenario_user_args_override = true
	var options := {"project_args": user_args}
	if scenario.get("determinism", {}) is Dictionary:
		options["determinism"] = (scenario.get("determinism") as Dictionary).duplicate(true)
	return _invoke_qa_apply_options(options)


func _invoke_qa_apply_options(options: Dictionary) -> bool:
	# Use the main loop rather than self.get_tree(): apply_scenario_options runs
	# before the scene loads (and, on the cold one-shot, before this bridge is
	# fully in a running tree), where Node.get_tree() would log an engine error.
	var loop := Engine.get_main_loop()
	if not loop is SceneTree:
		return false
	var root := (loop as SceneTree).get_root()
	if root == null:
		return false
	for child_index in range(root.get_child_count()):
		var child := root.get_child(child_index)
		if child == null or child == self:
			continue
		if child.has_method("qa_apply_options"):
			child.call("qa_apply_options", options.duplicate(true))
			return true
	return false

func configure_live_session(session_state: Dictionary) -> void:
	_live_session = session_state.duplicate(true)
	process_mode = Node.PROCESS_MODE_ALWAYS
	_live_edit = QaLiveEdit.new()
	_reinitialize_recorder()
	_record_log(
		"info",
		"session_started",
		"live session initialized",
		{
			"session_id": str(_live_session.get("session_id", "")),
			"scene": str(_live_session.get("scene", "")),
		},
	)


func _process(_delta: float) -> void:
	if _soak != null and str(_soak.status) == "running":
		_soak.sample()


func _prepare_android_live_session() -> void:
	if not FileAccess.file_exists(ANDROID_BOOTSTRAP_PATH):
		return

	var bootstrap_result := _read_android_bootstrap()
	# The bootstrap is a one-shot capability. Delete it before doing any work
	# which may await or fail so a later ordinary app launch cannot reactivate QA.
	DirAccess.remove_absolute(ProjectSettings.globalize_path(ANDROID_BOOTSTRAP_PATH))
	if not bool(bootstrap_result.get("ok", false)):
		_android_bootstrap_warning(str(bootstrap_result.get("reason", "invalid_bootstrap")))
		return
	var session: Dictionary = bootstrap_result.get("session", {})
	var project_args := PackedStringArray()
	for arg in session.get("project_args", []):
		project_args.append(str(arg))
	_scenario_user_args = project_args
	_scenario_user_args_override = true
	enabled = true
	activation_source = "android_bootstrap"
	_android_session_token = str(session.get("token", ""))
	_android_pending_session = session.duplicate(true)
	call_deferred("_start_android_live_session")


func _start_android_live_session() -> void:
	var session: Dictionary = _android_pending_session
	_android_pending_session = {}
	var session_id := str(session.get("id", ""))
	var artifact_root := "%s/%s" % [ANDROID_ARTIFACTS_PREFIX, session_id]
	var artifact_paths := {
		"root": artifact_root,
		"snapshots_dir": "%s/snapshots" % artifact_root,
		"screenshots_dir": "%s/screenshots" % artifact_root,
		"audio_dir": "%s/audio" % artifact_root,
	}
	for path in artifact_paths.values():
		var dir_error := DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(str(path)))
		if dir_error != OK:
			_android_bootstrap_fatal("artifact_directory_failed", "Android live artifact directory could not be created")
			return

	var mode := str(session.get("mode", "live"))
	var scene_path := str(session.get("scene", ""))
	if scene_path != "":
		var packed_scene := ResourceLoader.load(scene_path, "PackedScene")
		if packed_scene == null or not packed_scene is PackedScene:
			_android_bootstrap_fatal("scene_load_failed", "Android live scene could not be loaded")
			return
		var scene_error := get_tree().change_scene_to_packed(packed_scene)
		if scene_error != OK:
			_android_bootstrap_fatal("scene_change_failed", "Android live scene could not replace the packaged main scene")
			return
		await get_tree().process_frame

	await get_tree().process_frame
	RenderingServer.force_draw(false)
	await get_tree().process_frame
	if mode == "scenario" and str(session.get("scene", "")) == "":
		# Android analogue of the desktop warm sceneless boot: drop the packaged
		# main scene after it has booted so injected scenario input cannot reach the
		# game's menu/main scene. QaScenarioExecutor owns the per-scenario scene.
		var current_scene := get_tree().current_scene
		if current_scene != null:
			get_tree().current_scene = null
			current_scene.queue_free()
		scene_path = ""
	elif scene_path == "" and get_tree().current_scene != null:
		scene_path = str(get_tree().current_scene.scene_file_path)
	var viewport_size := get_viewport().get_visible_rect().size
	configure_live_session(
		{
			"session_id": session_id,
			"target": "android",
			"mode": mode,
			"scene": scene_path,
			"viewport": [int(viewport_size.x), int(viewport_size.y)],
			"started_at": str(session.get("started_at", "")),
			"artifacts_root": str(artifact_paths.get("root", "")),
			"snapshots_dir": str(artifact_paths.get("snapshots_dir", "")),
			"screenshots_dir": str(artifact_paths.get("screenshots_dir", "")),
			"audio_dir": str(artifact_paths.get("audio_dir", "")),
			"recorder_enabled": bool(session.get("recorder_enabled", false)),
			"recorder": session.get("recorder", {}),
		},
	)
	_invoke_qa_apply_options({"project_args": QaArgs.effective_user_args()})

	_android_live_host = QaLiveSessionHost.new()
	_android_live_host.name = "GodotQaLiveSessionHost"
	get_tree().get_root().add_child(_android_live_host)
	var host_result: Dictionary = _android_live_host.configure(
		self,
		int(session.get("port", 0)),
		"",
		false,
	)
	if not bool(host_result.get("ok", false)):
		_android_bootstrap_fatal("session_listen_failed", "Android live localhost server could not start")
		return
	print("[godot-qa] Android live session enabled")
	await _android_live_host.serve()
	QaRuntime.clear_provider_registry()
	enabled = false
	_android_session_token = ""
	_scenario_user_args_override = false
	_android_pending_session.clear()
	_live_session.clear()
	get_tree().quit(0)


func _read_android_bootstrap() -> Dictionary:
	var file := FileAccess.open(ANDROID_BOOTSTRAP_PATH, FileAccess.READ)
	if file == null:
		return {"ok": false, "reason": "bootstrap_read_failed"}
	var parser := JSON.new()
	if parser.parse(file.get_as_text()) != OK or not parser.data is Dictionary:
		return {"ok": false, "reason": "bootstrap_json_invalid"}
	var bootstrap: Dictionary = parser.data
	if not _dictionary_has_only_keys(bootstrap, ["version", "session"]):
		return {"ok": false, "reason": "bootstrap_fields_invalid"}
	var raw_version = bootstrap.get("version", null)
	if not (raw_version is int or raw_version is float) or int(raw_version) != ANDROID_BOOTSTRAP_VERSION or float(raw_version) != float(int(raw_version)):
		return {"ok": false, "reason": "bootstrap_version_invalid"}
	if not bootstrap.get("session", null) is Dictionary:
		return {"ok": false, "reason": "bootstrap_session_invalid"}
	var session: Dictionary = bootstrap.get("session", {})
	if not _dictionary_has_only_keys(
		session,
		["id", "token", "port", "scene", "started_at", "project_args", "recorder_enabled", "recorder", "mode"],
	):
		return {"ok": false, "reason": "bootstrap_session_fields_invalid"}
	if not session.get("id", null) is String or not _is_safe_android_session_id(str(session.get("id", ""))):
		return {"ok": false, "reason": "bootstrap_session_id_invalid"}
	if not session.get("token", null) is String or str(session.get("token", "")) == "":
		return {"ok": false, "reason": "bootstrap_token_invalid"}
	var raw_port = session.get("port", null)
	if not (raw_port is int or raw_port is float) or float(raw_port) != float(int(raw_port)):
		return {"ok": false, "reason": "bootstrap_port_invalid"}
	var port := int(raw_port)
	if port < 1 or port > 65535:
		return {"ok": false, "reason": "bootstrap_port_invalid"}
	var scene = session.get("scene", "")
	if not scene is String or not _is_safe_android_scene_path(str(scene)):
		return {"ok": false, "reason": "bootstrap_scene_invalid"}
	var started_at = session.get("started_at", "")
	if not started_at is String:
		return {"ok": false, "reason": "bootstrap_started_at_invalid"}
	var project_args = session.get("project_args", [])
	if not project_args is Array:
		return {"ok": false, "reason": "bootstrap_project_args_invalid"}
	for arg in project_args:
		if not arg is String:
			return {"ok": false, "reason": "bootstrap_project_args_invalid"}
	var recorder_enabled = session.get("recorder_enabled", false)
	if not recorder_enabled is bool:
		return {"ok": false, "reason": "bootstrap_recorder_invalid"}
	var recorder = session.get("recorder", {})
	if not recorder is Dictionary:
		return {"ok": false, "reason": "bootstrap_recorder_invalid"}
	# An absent mode defaults to the historical live session; only "live" and
	# "scenario" are valid. The resolved value is stored on the returned session so
	# the rest of the bridge can query it without re-deriving the default.
	var mode = session.get("mode", "live")
	if not mode is String or not (str(mode) == "live" or str(mode) == "scenario"):
		return {"ok": false, "reason": "bootstrap_mode_invalid"}
	var resolved := session.duplicate(true)
	resolved["mode"] = str(mode)
	return {"ok": true, "session": resolved}


func _dictionary_has_only_keys(value: Dictionary, allowed_keys: Array) -> bool:
	for key in value.keys():
		if not key is String or not allowed_keys.has(str(key)):
			return false
	return true


func _is_safe_android_session_id(session_id: String) -> bool:
	if session_id.length() < 1 or session_id.length() > 128:
		return false
	for character in session_id:
		var codepoint := character.unicode_at(0)
		if not (
			(codepoint >= 48 and codepoint <= 57)
			or (codepoint >= 65 and codepoint <= 90)
			or (codepoint >= 97 and codepoint <= 122)
			or character in ["-", "_"]
		):
			return false
	return true


func _is_safe_android_scene_path(scene_path: String) -> bool:
	if scene_path == "":
		return true
	return (
		scene_path.begins_with("res://")
		and (scene_path.ends_with(".tscn") or scene_path.ends_with(".scn"))
		and not ".." in scene_path
		and not "\\" in scene_path
	)


func _android_bootstrap_warning(reason: String) -> void:
	push_warning("[godot-qa] Android live bootstrap ignored (%s)" % reason)


func _android_bootstrap_fatal(reason: String, message: String) -> void:
	push_error("[godot-qa] %s (%s)" % [message, reason])
	enabled = false
	_android_session_token = ""
	_scenario_user_args_override = false
	_android_pending_session.clear()
	_live_session.clear()
	get_tree().quit(1)

func consume_stop_requested() -> bool:
	if not _stop_requested:
		return false
	_stop_requested = false
	return true

func _request_id_from_command(command: Dictionary) -> String:
	if not command.has("id"):
		return ""
	var request_id = command.get("id", "")
	if request_id == null:
		return ""
	return str(request_id)

func _configured_session_token() -> String:
	if _android_session_token != "":
		return _android_session_token
	return str(runtime_options.get("session_token", ""))

func _sanitized_command(command: Dictionary) -> Dictionary:
	var sanitized := command.duplicate(true)
	sanitized.erase("token")
	return sanitized


func _constant_time_equals(a: String, b: String) -> bool:
	# Compare tokens without an early-out so match/mismatch take the same time,
	# avoiding a timing side channel that could let a local attacker recover the
	# session token byte by byte. The length difference is folded into the
	# accumulator so mismatched lengths still fail.
	var a_bytes := a.to_utf8_buffer()
	var b_bytes := b.to_utf8_buffer()
	var max_len := maxi(a_bytes.size(), b_bytes.size())
	var diff := a_bytes.size() ^ b_bytes.size()
	for i in range(max_len):
		var a_byte: int = a_bytes[i] if i < a_bytes.size() else 0
		var b_byte: int = b_bytes[i] if i < b_bytes.size() else 0
		diff |= a_byte ^ b_byte
	return diff == 0

func handle_command(command: Dictionary) -> Dictionary:
	var request_id := _request_id_from_command(command)
	if not enabled:
		return QaProtocol.error(
			"bridge_inactive",
			"QA mode is not enabled",
			{"command": _sanitized_command(command)},
			request_id,
		)
	var expected_token := _configured_session_token()
	if expected_token == "":
		return QaProtocol.error(
			"session_token_unconfigured",
			"QA bridge requires --qa-session-token for live commands",
			{"command": _sanitized_command(command)},
			request_id,
		)
	var command_token := str(command.get("token", ""))
	if command_token == "":
		return QaProtocol.error(
			"session_token_required",
			"QA bridge command requires a session token",
			{"command": _sanitized_command(command)},
			request_id,
		)
	if not _constant_time_equals(command_token, expected_token):
		return QaProtocol.error(
			"invalid_session_token",
			"QA bridge session token did not match",
			{"command": _sanitized_command(command)},
			request_id,
		)
	var command_name := str(command.get("command", ""))
	# Bounded FIFO queue: overlapping commands wait their turn rather than being
	# rejected. We only fall back to the compatible `session_bridge_busy` error
	# when the queue is saturated or a command waits longer than the bound.
	if _bridge_queue.size() >= BRIDGE_QUEUE_MAX_DEPTH:
		return _bridge_busy_error(command_name, request_id, "queue_full", 0)
	var ticket := _bridge_queue_next_ticket
	_bridge_queue_next_ticket += 1
	_bridge_queue.append(ticket)
	var enqueued_msec := Time.get_ticks_msec()
	while _bridge_command_in_progress or _bridge_queue.is_empty() or _bridge_queue[0] != ticket:
		if not enabled:
			_bridge_dequeue_ticket(ticket)
			return QaProtocol.error(
				"bridge_inactive",
				"QA mode is not enabled",
				{"command": command_name, "request_id": request_id},
				request_id,
			)
		await get_tree().process_frame
		var waited_msec := max(0, Time.get_ticks_msec() - enqueued_msec)
		if waited_msec > BRIDGE_QUEUE_MAX_WAIT_MSEC:
			_bridge_dequeue_ticket(ticket)
			return _bridge_busy_error(command_name, request_id, "queue_wait_timeout", waited_msec)
	# Our turn: we are at the head of the queue and nothing else is in flight.
	# Claiming the lock is synchronous (no await before the assignment), so no
	# other queued coroutine can interleave ahead of us this frame.
	_bridge_command_in_progress = true
	_bridge_command_diagnostics = {
		"command": command_name,
		"request_id": request_id,
		"started_msec": Time.get_ticks_msec(),
	}
	if command_name == "session.wait_frames":
		var params = command.get("params", {})
		if params is Dictionary:
			_bridge_command_diagnostics["frames"] = max(0, int((params as Dictionary).get("frames", 0)))
	if command_name in ["trace.record", "event.trace", "telemetry.record", "session.profile"]:
		var params = command.get("params", {})
		if params is Dictionary:
			_bridge_command_diagnostics["frames"] = max(0, int((params as Dictionary).get("frames", 0)))
			_bridge_command_diagnostics["seconds"] = max(0.0, float((params as Dictionary).get("seconds", 0.0)))
	if command_name == "audio.capture" or command_name == "audio.render":
		var params = command.get("params", {})
		if params is Dictionary:
			_bridge_command_diagnostics["duration_seconds"] = max(0.0, float((params as Dictionary).get("duration_seconds", 0.0)))
	var response: Dictionary = await _dispatch_authenticated_command(command, request_id, command_name)
	_bridge_command_in_progress = false
	_bridge_command_diagnostics = {}
	_bridge_dequeue_ticket(ticket)
	return response


func _bridge_dequeue_ticket(ticket: int) -> void:
	_bridge_queue.erase(ticket)


func _bridge_busy_error(command_name: String, request_id: String, reason: String, waited_msec: int) -> Dictionary:
	var in_progress: Dictionary = _bridge_command_diagnostics.duplicate(true)
	if in_progress.has("started_msec"):
		in_progress["elapsed_msec"] = max(0, Time.get_ticks_msec() - int(in_progress.get("started_msec", 0)))
	return QaProtocol.error(
		"session_bridge_busy",
		"QA bridge queue is saturated; command was not accepted",
		{
			"command": command_name,
			"request_id": request_id,
			"reason": reason,
			"queue_depth": _bridge_queue.size(),
			"waited_msec": waited_msec,
			"in_progress": in_progress,
		},
		request_id,
	)


func _dispatch_authenticated_command(command: Dictionary, request_id: String, command_name: String) -> Dictionary:
	var runtime := QaRuntime.new(_resolve_scene_root(), get_tree().get_root(), _runtime_errors, _held_actions)
	var params := command.get("params", {})
	var live_session_enabled := not _live_session.is_empty()
	if live_session_enabled and command_name == "session.status":
		return QaProtocol.ok(_session_status_payload(), request_id)
	if live_session_enabled and command_name == "session.pause":
		return _handle_session_pause(request_id)
	if live_session_enabled and command_name == "session.resume":
		return _handle_session_resume(request_id)
	if live_session_enabled and command_name == "session.step":
		return await _handle_session_step(params, request_id)
	if live_session_enabled and command_name == "session.reload_scene":
		return await _handle_session_reload_scene(request_id)
	if live_session_enabled and command_name == "session.change_scene":
		return await _handle_session_change_scene(params, request_id)
	if live_session_enabled and command_name == "session.set_options":
		return _handle_session_set_options(params, request_id)
	if live_session_enabled and command_name == "scenario.run":
		if _is_android_live_session():
			# Live-mode Android sessions boot the packaged main scene, so scenario.run
			# stays unsupported. Scenario-mode sessions boot sceneless and accept it,
			# but only for a request_path beneath this session's artifacts root.
			if _android_live_session_mode() != "scenario":
				return QaProtocol.error(
					"android_command_unsupported",
					"scenario.run is not supported by Android live sessions",
					{"command": command_name, "target": "android", "mode": _android_live_session_mode()},
					request_id,
				)
			if not _android_request_path_in_artifacts_root(params):
				return QaProtocol.error(
					"invalid_params",
					"scenario.run request_path must resolve beneath the Android session artifacts root",
					{
						"command": command_name,
						"expected": "path beneath user://godot_qa/live/<session-id>",
						"actual": params,
					},
					request_id,
				)
		return await _handle_scenario_run(params, request_id)
	if live_session_enabled and command_name == "session.profile":
		return await _handle_session_profile(params, request_id)
	if live_session_enabled and command_name == "session.profile.soak_start":
		return _handle_soak_start(params, request_id)
	if live_session_enabled and command_name == "session.profile.soak_status":
		return _handle_soak_status(params, request_id)
	if live_session_enabled and command_name == "session.profile.soak_stop":
		return _handle_soak_stop(params, request_id)
	if live_session_enabled and command_name == "session.wait_frames":
		var frame_count = int(params.get("frames", 0))
		for _index in range(max(frame_count, 0)):
			await get_tree().process_frame
		if _recorder != null and _recorder.has_method("note_wait_frames"):
			_recorder.call("note_wait_frames", frame_count)
		_record_log(
			"info",
			"session_wait_frames",
			"processed requested frames",
			{"frames": frame_count},
		)
		return QaProtocol.ok({"frames": frame_count}, request_id)
	if live_session_enabled and command_name == "session.set_time_scale":
		return _handle_session_set_time_scale(params, request_id)
	if live_session_enabled and command_name == "session.stop":
		if _soak != null and str(_soak.status) == "running":
			_soak.stop()
		var roster_before: Array[int] = QaRuntime.effective_connected_joypads()
		var input_cleanup_failures: Array = await runtime.reset_input()
		var roster_after: Array[int] = QaRuntime.effective_connected_joypads()
		_stop_requested = true
		_record_log("info", "session_stop", "live session stop requested", {
			"input_cleanup_failures": input_cleanup_failures.size(),
		})
		return QaProtocol.ok({"stopping": true, "input_cleanup_failures": input_cleanup_failures,
			"joypad_roster_before": roster_before, "joypad_roster_after": roster_after}, request_id)
	if live_session_enabled and command_name == "snapshot.capture":
		var snapshot_name := _sanitize_capture_name(str(params.get("name", "current")))
		var snapshot_dir := "%s/%s" % [str(_live_session.get("snapshots_dir", "")), snapshot_name]
		var filter_params: Dictionary = params if params is Dictionary else {}
		filter_params = filter_params.duplicate(true)
		filter_params.erase("name")
		var snapshot_scope := runtime.resolve_snapshot_filter(filter_params)
		if not bool(snapshot_scope.get("ok", false)):
			var scope_error: Dictionary = snapshot_scope.get("error", {})
			return QaProtocol.error(
				str(scope_error.get("type", "invalid_snapshot_filter")),
				str(scope_error.get("message", "snapshot filter could not be resolved")),
				scope_error,
				request_id,
			)
		var capture_result: Dictionary = await QaSnapshot.capture_snapshot_bundle(
			snapshot_scope.get("root"),
			get_tree().get_root(),
			_runtime_errors,
			snapshot_dir,
			snapshot_scope.get("options", {}),
		)
		if bool(capture_result.get("ok", false)):
			if _recorder != null and _recorder.has_method("note_snapshot"):
				_recorder.call("note_snapshot", snapshot_name)
			_record_log(
				"info",
				"snapshot_capture",
				"captured live snapshot",
				{
					"name": snapshot_name,
					"artifacts": capture_result.get("artifacts", {}),
				},
			)
			return QaProtocol.ok(
				{
					"name": snapshot_name,
					"artifacts": capture_result.get("artifacts", {}),
				},
				request_id,
			)
		for failure in capture_result.get("failures", []):
			if failure is Dictionary:
				var failure_dict: Dictionary = (failure as Dictionary).duplicate(true)
				_record_runtime_error(str(failure_dict.get("message", "snapshot artifact failed")), "snapshot")
		var first_failure := {}
		var failures = capture_result.get("failures", [])
		if failures is Array and not failures.is_empty() and failures[0] is Dictionary:
			first_failure = (failures[0] as Dictionary).duplicate(true)
		first_failure["artifacts"] = capture_result.get("artifacts", {})
		return QaProtocol.error(
			str(first_failure.get("type", "snapshot_write_failed")),
			str(first_failure.get("message", "snapshot capture failed")),
			first_failure,
			request_id,
		)
	if live_session_enabled and command_name == "screenshot.capture":
		var screenshot_name := _sanitize_capture_name(str(params.get("name", "current")))
		var screenshots_dir := str(_live_session.get("screenshots_dir", ""))
		var screenshot_path := "%s/%s.png" % [screenshots_dir, screenshot_name]
		var screenshot_result: Dictionary = await QaSnapshot.capture_screenshot(
			get_tree().get_root(),
			screenshot_path,
			bool(params.get("allow_fallback", true)),
		)
		if bool(screenshot_result.get("ok", false)):
			var screenshot_data := {
				"name": screenshot_name,
				"artifacts": screenshot_result.get("artifacts", {}),
				"engine_frame": Engine.get_process_frames(),
				"time_msec": Time.get_ticks_msec(),
				"display_server": DisplayServer.get_name(),
				"session_id": str(_live_session.get("session_id", "")),
				"session_started_at": str(_live_session.get("started_at", "")),
				"session_scene": str(_live_session.get("scene", "")),
				"capture_source": "godot_viewport",
				"draw_forced": true,
				"allow_fallback": bool(params.get("allow_fallback", true)),
			}
			_record_log(
				"info",
				"screenshot_capture",
				"captured live screenshot",
				{
					"name": screenshot_name,
					"artifacts": screenshot_result.get("artifacts", {}),
				},
			)
			return QaProtocol.ok(screenshot_data, request_id)
		var screenshot_failure := {}
		var screenshot_failures = screenshot_result.get("failures", [])
		if screenshot_failures is Array and not screenshot_failures.is_empty() and screenshot_failures[0] is Dictionary:
			screenshot_failure = (screenshot_failures[0] as Dictionary).duplicate(true)
		screenshot_failure["artifacts"] = screenshot_result.get("artifacts", {})
		_record_runtime_error(str(screenshot_failure.get("message", "screenshot capture failed")), "screenshot")
		return QaProtocol.error(
			str(screenshot_failure.get("type", "screenshot_write_failed")),
			str(screenshot_failure.get("message", "screenshot capture failed")),
			screenshot_failure,
			request_id,
		)
	if live_session_enabled and (command_name == "audio.capture" or command_name == "audio.export" or command_name == "audio.render"):
		var audio_params: Dictionary = params if params is Dictionary else {}
		audio_params = audio_params.duplicate(true)
		var audio_name := _sanitize_capture_name(str(audio_params.get("name", "capture")))
		var audio_dir := str(_live_session.get("audio_dir", ""))
		audio_params["audio_dir"] = audio_dir
		audio_params["name"] = audio_name
		if _is_android_live_session():
			audio_params.erase("output_path")
			audio_params["wav_path"] = "%s/%s.wav" % [audio_dir, audio_name]
		elif not audio_params.has("wav_path"):
			audio_params["wav_path"] = str(audio_params.get("output_path", "%s/%s.wav" % [audio_dir, audio_name]))
		if not audio_params.has("max_bytes"):
			audio_params["max_bytes"] = 10485760
		var audio_result: Dictionary = await runtime.execute(command_name, audio_params)
		if bool(audio_result.get("ok", false)):
			var audio_data: Dictionary = audio_result.get("data", {}) if audio_result.get("data", {}) is Dictionary else {}
			_record_log(
				"info",
				"live_audio",
				"live audio command completed",
				{
					"command": command_name,
					"name": audio_name,
					"artifacts": audio_data.get("artifacts", {}),
					"warnings": audio_data.get("warnings", []),
				},
			)
			return QaProtocol.ok(audio_data, request_id)
		var audio_error: Dictionary = audio_result.get("error", {}) if audio_result.get("error", {}) is Dictionary else {}
		var audio_payload: Dictionary = audio_error.get("data", {}) if audio_error.get("data", {}) is Dictionary else {}
		_record_log(
			"error",
			"live_audio",
			str(audio_error.get("message", "live audio command failed")),
			{
				"command": command_name,
				"type": str(audio_error.get("type", "runtime_error")),
				"data": audio_payload,
			},
		)
		return QaProtocol.error(
			str(audio_error.get("type", "runtime_error")),
			str(audio_error.get("message", "live audio command failed")),
			audio_payload,
			request_id,
		)
	if live_session_enabled and command_name == "logs.read":
		return QaProtocol.ok(_read_logs(str(params.get("since", ""))), request_id)
	if live_session_enabled and command_name == "record.export":
		if _recorder != null and _recorder.has_method("export"):
			return QaProtocol.ok(_recorder.call("export"), request_id)
		return QaProtocol.error(
			"unsupported_command",
			"QA bridge command is not supported in this milestone",
			{"command": _sanitized_command(command)},
			request_id,
		)
	if live_session_enabled and command_name == "edit.set_property":
		var live_edit_result: Dictionary = await _live_edit.set_property(
			runtime,
			_resolve_scene_root(),
			str(params.get("target", "")),
			str(params.get("property", "")),
			str(params.get("value_input", "")),
		)
		return _live_edit_protocol_response(command_name, live_edit_result, request_id)
	if live_session_enabled and command_name == "edit.call_method":
		var live_call_result: Dictionary = await _live_edit.call_method(
			runtime,
			_resolve_scene_root(),
			str(params.get("target", "")),
			str(params.get("method", "")),
		)
		return _live_edit_protocol_response(command_name, live_call_result, request_id)
	if live_session_enabled and command_name == "edit.diff":
		return _live_edit_protocol_response(command_name, _live_edit.diff(runtime), request_id)
	if live_session_enabled and command_name == "edit.export_patch":
		return _live_edit_protocol_response(command_name, _live_edit.export_patch(runtime, _resolve_scene_root()), request_id)
	if live_session_enabled and command_name == "physics.inspect":
		var physics_result: Dictionary = await runtime.execute(command_name, params)
		if not bool(physics_result.get("ok", false)):
			var physics_error_data = physics_result.get("error", {})
			if not physics_error_data is Dictionary:
				physics_error_data = {}
			return QaProtocol.error(
				str(physics_error_data.get("type", "runtime_error")),
				str(physics_error_data.get("message", "Runtime command failed")),
				physics_error_data.get("data", {}),
				request_id,
			)
		var physics_data: Dictionary = physics_result.get("data", {})
		var snapshot_name := str((params as Dictionary).get("snapshot", "")) if params is Dictionary else ""
		if snapshot_name != "":
			var snapshot_dir := "%s/%s" % [str(_live_session.get("snapshots_dir", "")), _sanitize_capture_name(snapshot_name)]
			var capture_result: Dictionary = await QaSnapshot.capture_snapshot_bundle(
				_resolve_scene_root(),
				get_tree().get_root(),
				_runtime_errors,
				snapshot_dir,
				{},
			)
			if bool(capture_result.get("ok", false)):
				physics_data["snapshot"] = _sanitize_capture_name(snapshot_name)
				var artifacts: Dictionary = physics_data.get("artifacts", {}) if physics_data.get("artifacts", {}) is Dictionary else {}
				artifacts["snapshot"] = capture_result.get("artifacts", {})
				physics_data["artifacts"] = artifacts
			else:
				var snapshot_failures = capture_result.get("failures", [])
				var snapshot_failure := {}
				if snapshot_failures is Array and not snapshot_failures.is_empty() and snapshot_failures[0] is Dictionary:
					snapshot_failure = (snapshot_failures[0] as Dictionary).duplicate(true)
				return QaProtocol.error(
					str(snapshot_failure.get("type", "snapshot_write_failed")),
					str(snapshot_failure.get("message", "physics inspect snapshot capture failed")),
					snapshot_failure,
					request_id,
				)
		_record_log("info", "physics_inspect", "captured physics inspection", {"target": physics_data.get("target", "")})
		return QaProtocol.ok(physics_data, request_id)
	if live_session_enabled and command_name == "trace.record":
		var trace_params: Dictionary = params if params is Dictionary else {}
		var trace_artifacts := {}
		if bool(trace_params.get("snapshot_start", false)):
			var start_dir := "%s/%s" % [str(_live_session.get("snapshots_dir", "")), _sanitize_capture_name("trace-start")]
			var start_capture: Dictionary = await QaSnapshot.capture_snapshot_bundle(
				_resolve_scene_root(),
				get_tree().get_root(),
				_runtime_errors,
				start_dir,
				{},
			)
			if bool(start_capture.get("ok", false)):
				trace_artifacts["snapshot_start"] = start_capture.get("artifacts", {})
			else:
				var start_failures = start_capture.get("failures", [])
				var start_failure := {}
				if start_failures is Array and not start_failures.is_empty() and start_failures[0] is Dictionary:
					start_failure = (start_failures[0] as Dictionary).duplicate(true)
				return QaProtocol.error(
					str(start_failure.get("type", "snapshot_write_failed")),
					str(start_failure.get("message", "trace start snapshot capture failed")),
					start_failure,
					request_id,
				)
		var trace_result: Dictionary = await runtime.execute(command_name, trace_params)
		if not bool(trace_result.get("ok", false)):
			var trace_error_data = trace_result.get("error", {})
			if not trace_error_data is Dictionary:
				trace_error_data = {}
			var trace_error_payload: Dictionary = trace_error_data.get("data", {}) if trace_error_data.get("data", {}) is Dictionary else {}
			trace_error_payload = trace_error_payload.duplicate(true)
			trace_error_payload["runtime_error"] = (trace_error_data as Dictionary).duplicate(true)
			_record_log("error", "trace_record", str((trace_error_data as Dictionary).get("message", "Runtime command failed")), {
				"type": str((trace_error_data as Dictionary).get("type", "runtime_error")),
				"data": trace_error_payload,
			})
			return QaProtocol.error(
				str(trace_error_data.get("type", "runtime_error")),
				str(trace_error_data.get("message", "Runtime command failed")),
				trace_error_payload,
				request_id,
			)
		var trace_data: Dictionary = trace_result.get("data", {})
		if bool(trace_params.get("snapshot_end", false)):
			var end_dir := "%s/%s" % [str(_live_session.get("snapshots_dir", "")), _sanitize_capture_name("trace-end")]
			var end_capture: Dictionary = await QaSnapshot.capture_snapshot_bundle(
				_resolve_scene_root(),
				get_tree().get_root(),
				_runtime_errors,
				end_dir,
				{},
			)
			if bool(end_capture.get("ok", false)):
				trace_artifacts["snapshot_end"] = end_capture.get("artifacts", {})
			else:
				var end_failures = end_capture.get("failures", [])
				var end_failure := {}
				if end_failures is Array and not end_failures.is_empty() and end_failures[0] is Dictionary:
					end_failure = (end_failures[0] as Dictionary).duplicate(true)
				return QaProtocol.error(
					str(end_failure.get("type", "snapshot_write_failed")),
					str(end_failure.get("message", "trace end snapshot capture failed")),
					end_failure,
					request_id,
				)
		if not trace_artifacts.is_empty():
			var existing_artifacts: Dictionary = trace_data.get("artifacts", {}) if trace_data.get("artifacts", {}) is Dictionary else {}
			for key in trace_artifacts.keys():
				existing_artifacts[key] = trace_artifacts[key]
			trace_data["artifacts"] = existing_artifacts
		_record_log("info", "trace_record", "captured runtime trace", {"samples": (trace_data.get("samples", []) as Array).size() if trace_data.get("samples", []) is Array else 0})
		return QaProtocol.ok(trace_data, request_id)
	if live_session_enabled and _is_temporal_assert_command(command_name):
		var temporal_params: Dictionary = params if params is Dictionary else {}
		temporal_params = temporal_params.duplicate(true)
		temporal_params["temporal_dir"] = "%s/temporal" % str(_live_session.get("artifacts_root", "user://qa_artifacts"))
		params = temporal_params
	if live_session_enabled and _is_visual_assert_command(command_name):
		var visual_params: Dictionary = params if params is Dictionary else {}
		visual_params = visual_params.duplicate(true)
		visual_params["visual_dir"] = "%s/visual" % str(_live_session.get("artifacts_root", "user://qa_artifacts"))
		params = visual_params
	var result: Dictionary = await runtime.execute(command_name, params)
	if live_session_enabled and _recorder != null and _recorder.has_method("note_runtime_command"):
		if RECORDED_INPUT_COMMANDS.has(command_name):
			_recorder.call("note_runtime_command", command_name, params, result)
	if not bool(result.get("ok", false)):
		_record_log(
			"error",
			"runtime_error",
			str((result.get("error", {}) as Dictionary).get("message", "Runtime command failed")),
			(result.get("error", {}) as Dictionary).get("data", {}),
		)
	else:
		_record_log("info", "runtime_command", "live command completed", {"command": command_name})
	if bool(result.get("ok", false)):
		if command_name == "input.click_target" and _recorder != null and _recorder.has_method("note_click_target"):
			var result_data := (result.get("data", {}) as Dictionary).duplicate(true)
			_recorder.call(
				"note_click_target",
				str(result_data.get("target", str(params.get("target", "")))),
				str(result_data.get("resolved", "")),
			)
		return QaProtocol.ok(result.get("data", {}), request_id)

	var error_data = result.get("error", {})
	if not error_data is Dictionary:
		error_data = {}
	var error_payload = error_data.get("data", {})
	if str(error_data.get("type", "")) == "unsupported_command":
		error_payload = {"command": _sanitized_command(command)}
	return QaProtocol.error(
		str(error_data.get("type", "runtime_error")),
		str(error_data.get("message", "Runtime command failed")),
		error_payload,
		request_id,
	)


func _resolve_scene_root() -> Node:
	var tree := get_tree()
	if tree == null:
		return null
	if tree.current_scene != null:
		return tree.current_scene

	var root := tree.get_root()
	for child_index in range(root.get_child_count() - 1, -1, -1):
		var child := root.get_child(child_index) as Node
		if child == null:
			continue
		if child == self:
			continue
		if str(child.name) in ["GodotQaBridge", "GodotQaRecorder", "GodotQaLiveSessionHost"]:
			continue
		return child
	return null

func _sanitize_capture_name(raw_name: String) -> String:
	var input_name := raw_name.strip_edges().to_lower()
	if input_name == "":
		return "current"
	var sanitized := ""
	for character in input_name:
		var codepoint := character.unicode_at(0)
		var keep := (
			(codepoint >= 48 and codepoint <= 57)
			or (codepoint >= 97 and codepoint <= 122)
			or character in ["-", "_"]
		)
		sanitized += character if keep else "-"
	var collapsed: PackedStringArray = []
	for segment in sanitized.split("-"):
		if segment != "":
			collapsed.append(segment)
	return "-".join(collapsed) if not collapsed.is_empty() else "current"

func _session_status_payload() -> Dictionary:
	var viewport = _live_session.get("viewport", [])
	if _is_android_live_session() and get_viewport() != null:
		var visible_size := get_viewport().get_visible_rect().size
		viewport = [int(visible_size.x), int(visible_size.y)]
	return {
		"session_id": str(_live_session.get("session_id", "")),
		"target": str(_live_session.get("target", "desktop")),
		"scene": str(_live_session.get("scene", "")),
		"viewport": viewport,
		"started_at": str(_live_session.get("started_at", "")),
		"display_server": DisplayServer.get_name(),
		"stop_requested": _stop_requested,
		"paused": _is_session_paused(),
		"time_scale": Engine.time_scale,
		"vsync_mode": DisplayServer.window_get_vsync_mode(),
		"renderer": RenderingServer.get_current_rendering_method(),
		"rendering_driver": RenderingServer.get_current_rendering_driver_name(),
		"window_size": [DisplayServer.window_get_size().x, DisplayServer.window_get_size().y],
	}


func _is_android_live_session() -> bool:
	return str(_live_session.get("target", "desktop")) == "android"


func _android_live_session_mode() -> String:
	return str(_live_session.get("mode", "live"))


# Whether an Android scenario.run request_path resolves beneath this session's private
# artifacts root (user://godot_qa/live/<session-id>). Traversal is normalized away first.
func _android_request_path_in_artifacts_root(params: Variant) -> bool:
	if not params is Dictionary:
		return false
	var request_path := str((params as Dictionary).get("request_path", ""))
	var root := str(_live_session.get("artifacts_root", ""))
	if request_path == "" or root == "":
		return false
	var normalized := request_path.simplify_path()
	var normalized_root := root.simplify_path()
	return normalized == normalized_root or normalized.begins_with("%s/" % normalized_root)


func _is_session_paused() -> bool:
	var tree := get_tree()
	return tree != null and tree.paused


func _handle_session_pause(request_id: String) -> Dictionary:
	var tree := get_tree()
	if tree == null:
		return QaProtocol.error(
			"scene_unavailable",
			"QA runtime scene is not ready",
			{"command": "session.pause"},
			request_id,
		)
	if tree.paused:
		return QaProtocol.error(
			"session_already_paused",
			"live session is already paused",
			{
				"command": "session.pause",
				"expected": "running live session",
				"actual": "paused",
				"target": "session",
			},
			request_id,
		)
	tree.paused = true
	_record_log("info", "session_pause", "live session paused", {"paused": true})
	return QaProtocol.ok({"paused": true}, request_id)


func _handle_session_resume(request_id: String) -> Dictionary:
	var tree := get_tree()
	if tree == null:
		return QaProtocol.error(
			"scene_unavailable",
			"QA runtime scene is not ready",
			{"command": "session.resume"},
			request_id,
		)
	if not tree.paused:
		return QaProtocol.error(
			"session_not_paused",
			"live session is not paused",
			{
				"command": "session.resume",
				"expected": "paused live session",
				"actual": "running",
				"target": "session",
			},
			request_id,
		)
	tree.paused = false
	_record_log("info", "session_resume", "live session resumed", {"paused": false})
	return QaProtocol.ok({"paused": false}, request_id)


func _handle_session_step(params: Variant, request_id: String) -> Dictionary:
	var tree := get_tree()
	if tree == null:
		return QaProtocol.error(
			"scene_unavailable",
			"QA runtime scene is not ready",
			{"command": "session.step"},
			request_id,
		)
	var command_params: Dictionary = params if params is Dictionary else {}
	var frame_count := _coerce_positive_frame_count(command_params.get("frames", null))
	if frame_count == null:
		return QaProtocol.error(
			"invalid_params",
			"session.step requires a positive integer frame count",
			{
				"command": "session.step",
				"expected": "positive integer frames",
				"actual": command_params.get("frames", null),
			},
			request_id,
		)
	if not tree.paused:
		return QaProtocol.error(
			"session_not_paused",
			"live session must be paused before stepping frames",
			{
				"command": "session.step",
				"expected": "paused live session",
				"actual": "running",
				"target": "session",
			},
			request_id,
		)
	tree.paused = false
	for _index in range(frame_count):
		await tree.process_frame
	RenderingServer.force_draw(false)
	tree.paused = true
	_record_log(
		"info",
		"session_step",
		"processed requested paused-session frames",
		{"frames": frame_count, "paused": true},
	)
	return QaProtocol.ok({"paused": true, "frames": frame_count}, request_id)


func _handle_session_set_time_scale(params: Variant, request_id: String) -> Dictionary:
	var command_params: Dictionary = params if params is Dictionary else {}
	var time_scale_value = command_params.get("time_scale", null)
	var time_scale := _coerce_time_scale(time_scale_value)
	if time_scale == null:
		return QaProtocol.error(
			"invalid_params",
			"session.set_time_scale requires time_scale greater than 0.0 and <= 64",
			{
				"command": "session.set_time_scale",
				"expected": "positive finite number <= 64",
				"actual": time_scale_value,
			},
			request_id,
		)

	var physics_ticks_per_second := _coerce_optional_positive_integer(command_params.get("physics_ticks_per_second", null))
	if command_params.has("physics_ticks_per_second") and physics_ticks_per_second == null:
		return QaProtocol.error(
			"invalid_params",
			"session.set_time_scale requires physics_ticks_per_second >= 1",
			{
				"command": "session.set_time_scale",
				"expected": "integer >= 1",
				"actual": command_params.get("physics_ticks_per_second", null),
			},
			request_id,
		)
	var max_physics_steps_per_frame := _coerce_optional_positive_integer(command_params.get("max_physics_steps_per_frame", null))
	if command_params.has("max_physics_steps_per_frame") and max_physics_steps_per_frame == null:
		return QaProtocol.error(
			"invalid_params",
			"session.set_time_scale requires max_physics_steps_per_frame >= 1",
			{
				"command": "session.set_time_scale",
				"expected": "integer >= 1",
				"actual": command_params.get("max_physics_steps_per_frame", null),
			},
			request_id,
		)

	var original := _engine_time_settings_snapshot()
	var requested := {"time_scale": time_scale}
	if not is_equal_approx(float(time_scale), 1.0) and _soak != null and str(_soak.status) == "running":
		_soak.stop()
	Engine.time_scale = time_scale
	if physics_ticks_per_second != null:
		requested["physics_ticks_per_second"] = physics_ticks_per_second
		Engine.physics_ticks_per_second = physics_ticks_per_second
	if max_physics_steps_per_frame != null:
		requested["max_physics_steps_per_frame"] = max_physics_steps_per_frame
		Engine.max_physics_steps_per_frame = max_physics_steps_per_frame

	var warnings: Array = []
	if time_scale > 1.0 and (physics_ticks_per_second == null or max_physics_steps_per_frame == null):
		warnings.append({
			"type": "time_control_physics_pairing_missing",
			"message": "live time_scale acceleration was requested without both paired physics overrides",
			"target": "session",
		})
	var effective := _engine_time_settings_snapshot()
	var payload := {
		"original": original,
		"requested": requested,
		"effective": effective,
		"persistent": true,
		"warnings": warnings,
	}
	_record_log(
		"info",
		"session_set_time_scale",
		"live session Engine time settings changed",
		payload,
	)
	return QaProtocol.ok(payload, request_id)


func _handle_session_reload_scene(request_id: String) -> Dictionary:
	var tree := get_tree()
	if tree == null:
		return QaProtocol.error(
			"scene_unavailable",
			"QA runtime scene is not ready",
			{"command": "session.reload_scene"},
			request_id,
		)
	var scene_path := str(_live_session.get("scene", ""))
	if scene_path == "":
		return QaProtocol.error(
			"invalid_session_state",
			"live session does not have a scene path to reload",
			{
				"command": "session.reload_scene",
				"expected": "live session scene path",
				"actual": "missing",
			},
			request_id,
		)

	return await _swap_live_scene(scene_path, "session.reload_scene", request_id)


# Change the live session's active scene to a DIFFERENT scene path. Mirrors
# session.reload_scene but loads params.scene and updates _live_session.scene so
# session.status / reload stay consistent. The warm scenario.run path owns its
# own scene lifecycle via the shared executor; this is the standalone primitive
# and a diagnostic/fallback.
func _handle_session_change_scene(params: Variant, request_id: String) -> Dictionary:
	var command_params: Dictionary = params if params is Dictionary else {}
	var scene_path := str(command_params.get("scene", ""))
	if scene_path == "":
		return QaProtocol.error(
			"invalid_params",
			"session.change_scene requires a scene path",
			{
				"command": "session.change_scene",
				"expected": "non-empty scene path",
				"actual": command_params,
			},
			request_id,
		)
	_live_session["scene"] = scene_path
	return await _swap_live_scene(scene_path, "session.change_scene", request_id)


func _swap_live_scene(scene_path: String, command_name: String, request_id: String) -> Dictionary:
	if _soak != null and str(_soak.status) == "running":
		_soak.stop()
	var tree := get_tree()
	if tree == null:
		return QaProtocol.error(
			"scene_unavailable",
			"QA runtime scene is not ready",
			{"command": command_name},
			request_id,
		)
	var old_scene := _resolve_scene_root()
	var old_scene_path := str(old_scene.get_path()) if old_scene != null else ""
	var was_paused := tree.paused
	var cleanup_runtime := QaRuntime.new(old_scene, tree.get_root(), _runtime_errors, _held_actions)
	var input_cleanup_failures: Array = await cleanup_runtime.reset_input()
	if not input_cleanup_failures.is_empty():
		return QaProtocol.error(
			"input_cleanup_failed",
			"live scene could not safely release injected input before replacement",
			{"command": command_name, "scene": scene_path, "failures": input_cleanup_failures},
			request_id,
		)

	var packed_scene := ResourceLoader.load(
		scene_path,
		"PackedScene",
		ResourceLoader.CACHE_MODE_REPLACE_DEEP,
	)
	if packed_scene == null or not packed_scene is PackedScene:
		return QaProtocol.error(
			"scene_load_failed",
			"live session scene could not be loaded",
			{
				"command": command_name,
				"scene": scene_path,
				"expected": "loadable PackedScene",
				"actual": "missing_or_invalid",
				"old_root": old_scene_path,
			},
			request_id,
		)

	var new_scene: Node = packed_scene.instantiate()
	if new_scene == null:
		return QaProtocol.error(
			"scene_instantiate_failed",
			"live session scene could not be instantiated",
			{
				"command": command_name,
				"scene": scene_path,
				"expected": "instantiated scene root",
				"actual": "missing",
				"old_root": old_scene_path,
			},
			request_id,
		)

	tree.get_root().add_child(new_scene)
	tree.current_scene = new_scene
	var new_scene_path := str(new_scene.get_path())
	if old_scene != null and is_instance_valid(old_scene):
		old_scene.queue_free()
	_runtime_errors.clear()
	_live_edit = QaLiveEdit.new()
	_reinitialize_recorder()
	await tree.process_frame
	RenderingServer.force_draw(false)
	tree.paused = was_paused
	_record_log(
		"info",
		"session_swap_scene",
		"live session scene swapped",
		{
			"command": command_name,
			"scene": scene_path,
			"old_root": old_scene_path,
			"new_root": new_scene_path,
			"paused": was_paused,
		},
	)
	return QaProtocol.ok(
		{
			"scene": scene_path,
			"old_root": old_scene_path,
			"new_root": new_scene_path,
			"paused": was_paused,
		},
		request_id,
	)


# Explicitly push per-scenario QA options/project_args to the game between
# scenarios (used by tests/diagnostics). The warm scenario.run path applies
# options automatically via the shared executor, so this is mostly for manual
# control. Reports whether a cooperative qa_apply_options push hook was found.
func _handle_session_set_options(params: Variant, request_id: String) -> Dictionary:
	var command_params: Dictionary = params if params is Dictionary else {}
	var scenario := {}
	if command_params.get("project_args", []) is Array:
		scenario["project_args"] = command_params.get("project_args", [])
	if command_params.get("determinism", {}) is Dictionary:
		scenario["determinism"] = command_params.get("determinism", {})
	var hook_present := apply_scenario_options(scenario)
	_record_log(
		"info",
		"session_set_options",
		"applied per-scenario QA options",
		{"hook_present": hook_present},
	)
	return QaProtocol.ok(
		{
			"applied": true,
			"hook_present": hook_present,
			"project_args": _scenario_user_args,
		},
		request_id,
	)


# Run a full scenario in-process via the shared QaScenarioExecutor. The Rust
# warm runner passes the path to a request.json it already wrote (identical to
# cold). The executor loads the scene, runs steps/asserts, writes result.json to
# the per-scenario artifact dir, and self-cleans input/providers/engine settings
# on finalize. Rust reads result.json from disk; this response is liveness only.
#
# The optional `run_id` param is the CLI run id for this scenario. When present,
# the handler brackets the whole scenario with stderr marker lines so the CLI can
# slice this scenario's engine output out of the shared session log and report
# `data.logs` / `godot_log_error` / `godot_log_warning` exactly like a cold run.
# A cold run passes no `run_id`, so cold logs stay byte-identical.
func _handle_scenario_run(params: Variant, request_id: String) -> Dictionary:
	var command_params: Dictionary = params if params is Dictionary else {}
	var request_path := str(command_params.get("request_path", ""))
	var run_id := str(command_params.get("run_id", ""))
	# Opened before anything else this handler can log, and closed on EVERY exit
	# below, so the CLI always sees a well-formed window — even for an early
	# request error that never reached the executor.
	_print_scenario_log_marker("begin", run_id)
	if request_path == "":
		_print_scenario_log_marker("end", run_id)
		return QaProtocol.error(
			"invalid_params",
			"scenario.run requires a request_path to a written request.json",
			{
				"command": "scenario.run",
				"expected": "request_path string",
				"actual": command_params,
			},
			request_id,
		)
	var tree := get_tree()
	if tree == null:
		_print_scenario_log_marker("end", run_id)
		return QaProtocol.error(
			"scene_unavailable",
			"QA runtime scene is not ready",
			{"command": "scenario.run"},
			request_id,
		)
	var executor = QaScenarioExecutor.new(tree)
	var outcome: Dictionary = await executor.run_scenario(request_path)
	# Closed as soon as run_scenario returns: the executor already freed the scene
	# and awaited a frame inside _finalize_result_payload, so teardown errors are
	# inside the window. Anything the engine emits later is unattributed and stays
	# only in the session log.
	_print_scenario_log_marker("end", run_id)
	if str(outcome.get("status", "")) == "request_error":
		var err: Dictionary = (outcome.get("error", {}) as Dictionary).duplicate(true)
		err["id"] = request_id
		return err
	var payload: Dictionary = outcome.get("payload", {})
	var failures = payload.get("failures", [])
	_record_log(
		"info",
		"scenario_run",
		"warm scenario executed",
		{
			"scenario": str(payload.get("scenario", "")),
			"ok": bool(payload.get("ok", false)),
			"result_path": str(outcome.get("result_path", "")),
		},
	)
	return QaProtocol.ok(
		{
			"scenario": str(payload.get("scenario", "")),
			"ok": bool(payload.get("ok", false)),
			"write_ok": bool(outcome.get("write_ok", true)),
			"result_path": str(outcome.get("result_path", "")),
			"failure_count": failures.size() if failures is Array else 0,
		},
		request_id,
	)


# Print one warm scenario-log boundary marker. `printerr` (never `print`) so the
# marker rides the same unbuffered stderr stream engine `ERROR:`/`WARNING:` lines
# use — stdout is separately buffered, so a stdout marker could be flushed after
# an error it is supposed to precede. No run id means a cold/legacy caller: emit
# nothing at all so those logs stay byte-identical.
func _print_scenario_log_marker(edge: String, run_id: String) -> void:
	if run_id == "":
		return
	printerr("%s %s %s" % [WARM_LOG_MARKER_PREFIX, edge, run_id])


func _handle_soak_start(params: Variant, request_id: String) -> Dictionary:
	if _soak != null and str(_soak.status) == "running":
		return QaProtocol.error("soak_already_running", "one soak is already active in this session", {"run_id": str(_soak.run_id)}, request_id)
	if not params is Dictionary or not (params as Dictionary).get("spec", null) is Dictionary:
		return QaProtocol.error("soak_spec_invalid", "soak start requires a spec object", {"actual": params}, request_id)
	var id := str((params as Dictionary).get("run_id", ""))
	if id == "" or not id.is_valid_filename() or id.contains(".."):
		return QaProtocol.error("soak_run_id_invalid", "run id must be a safe filename", {"actual": id}, request_id)
	var root := str(_live_session.get("artifacts_root", ""))
	if root == "":
		return QaProtocol.error("artifact_root_missing", "live session has no artifact root", {}, request_id)
	_soak = QaSoak.new()
	var state_runtime := QaRuntime.new(_resolve_scene_root(), get_tree().get_root(), _runtime_errors, _held_actions)
	var result: Dictionary = _soak.start((params as Dictionary).get("spec", {}), state_runtime, root.path_join("profiles").path_join(id), id, "", Callable(), {"display_server": DisplayServer.get_name(), "vsync_mode": DisplayServer.window_get_vsync_mode(), "renderer": RenderingServer.get_current_rendering_method(), "rendering_driver": RenderingServer.get_current_rendering_driver_name(), "window_size": [DisplayServer.window_get_size().x, DisplayServer.window_get_size().y], "viewport_size": [get_viewport().get_visible_rect().size.x, get_viewport().get_visible_rect().size.y], "time_scale": Engine.time_scale, "max_fps": Engine.max_fps, "physics_ticks_per_second": Engine.physics_ticks_per_second, "pacing": "realtime", "instrumentation": "soak", "exclusions": ["warmup", "before_sampler_start", "after_sampler_stop"], "monitor_source": "cached_godot_performance", "monitor_every_frames": int((params as Dictionary).get("spec", {}).get("monitor_every_frames", 60))})
	if not bool(result.get("ok", false)):
		_soak = null
		return QaProtocol.error(str(result.get("type", "soak_start_failed")), "could not start soak", result, request_id)
	return QaProtocol.ok(result, request_id)


func _handle_soak_status(params: Variant, request_id: String) -> Dictionary:
	var id := str((params as Dictionary).get("run_id", "")) if params is Dictionary else ""
	if _soak == null or id != str(_soak.run_id):
		return QaProtocol.error("soak_not_found", "soak run is not active in this session", {"run_id": id}, request_id)
	return QaProtocol.ok(_soak.summary(), request_id)


func _handle_soak_stop(params: Variant, request_id: String) -> Dictionary:
	var id := str((params as Dictionary).get("run_id", "")) if params is Dictionary else ""
	if _soak == null or id != str(_soak.run_id):
		return QaProtocol.error("soak_not_found", "soak run is not active in this session", {"run_id": id}, request_id)
	return QaProtocol.ok(_soak.stop(), request_id)


func _handle_session_profile(params: Variant, request_id: String) -> Dictionary:
	var tree := get_tree()
	if tree == null:
		return QaProtocol.error(
			"scene_unavailable",
			"QA runtime scene is not ready",
			{"command": "session.profile"},
			request_id,
		)
	var command_params: Dictionary = params if params is Dictionary else {}
	var max_frame_samples := max(int(command_params.get("max_frame_samples", 120)), 0)
	var requested_top := max(int(command_params.get("top", 10)), 0)
	var requested_frames := int(command_params.get("frames", 0))
	var requested_seconds := float(command_params.get("seconds", 0.0))
	if requested_frames <= 0 and requested_seconds <= 0.0:
		requested_frames = 60

	var started_msec := Time.get_ticks_msec()
	var started_usec := Time.get_ticks_usec()
	var last_frame_usec := started_usec
	var frame_times_ms: Array = []
	var frame_samples: Array[Dictionary] = []
	var monitor_samples: Array[Dictionary] = []
	var unsupported_monitors: Array[String] = []
	var sampled_frames := 0
	QaProfiler.reset()

	while true:
		if requested_frames > 0 and sampled_frames >= requested_frames:
			break
		if requested_frames <= 0 and requested_seconds > 0.0:
			var elapsed_seconds := float(Time.get_ticks_usec() - started_usec) / 1000000.0
			if elapsed_seconds >= requested_seconds:
				break
		await tree.process_frame
		RenderingServer.force_draw(false)
		var now_usec := Time.get_ticks_usec()
		var elapsed_ms := float(max(now_usec - last_frame_usec, 0)) / 1000.0
		last_frame_usec = now_usec
		if frame_times_ms.size() < max_frame_samples:
			frame_times_ms.append(elapsed_ms)
			frame_samples.append({
				"index": frame_samples.size(),
				"timestamp_usec": now_usec,
				"duration_ms": elapsed_ms,
			})
		for monitor_name in PROFILE_MONITORS.keys():
			var monitor_id: int = PROFILE_MONITORS[monitor_name]
			var value = Performance.get_monitor(monitor_id)
			if value == null:
				if not unsupported_monitors.has(monitor_name):
					unsupported_monitors.append(monitor_name)
				continue
			if monitor_samples.size() < max_frame_samples * PROFILE_MONITORS.size():
				monitor_samples.append({
					"name": monitor_name,
					"value": float(value),
					"frame_index": sampled_frames,
					"timestamp_usec": now_usec,
				})
		sampled_frames += 1

	var duration_ms := Time.get_ticks_msec() - started_msec
	var node_process_counts := _node_process_counts()
	var resource_activity := _resource_activity_summary()
	var top_script_methods := QaProfiler.snapshot(requested_top, false)
	var payload := {
		"duration_ms": duration_ms,
		"frame_times_ms": frame_times_ms,
		"frame_samples": frame_samples,
		"monitor_samples": monitor_samples,
		"unsupported_monitors": unsupported_monitors,
		"node_process_counts": node_process_counts,
		"resource_activity": resource_activity,
		"top_script_methods": top_script_methods,
		"sample_window": {
			"requested_frames": requested_frames,
			"requested_seconds": requested_seconds,
			"sampled_frames": sampled_frames,
			"duration_ms": duration_ms,
		},
		"sample_limits": {
			"max_frame_samples": max_frame_samples,
			"top": requested_top,
		},
	}
	_record_log(
		"info",
		"session_profile",
		"captured live session profile",
		{
			"sampled_frames": sampled_frames,
			"duration_ms": duration_ms,
			"max_frame_samples": max_frame_samples,
		},
	)
	return QaProtocol.ok(payload, request_id)


func _node_process_counts() -> Dictionary:
	var root := get_tree().get_root()
	var stack: Array = [root]
	var total_nodes := 0
	var processing_nodes := 0
	var physics_processing_nodes := 0
	var internal_processing_nodes := 0
	var internal_physics_processing_nodes := 0
	while not stack.is_empty():
		var node := stack.pop_back() as Node
		if node == null:
			continue
		total_nodes += 1
		if node.is_processing():
			processing_nodes += 1
		if node.is_physics_processing():
			physics_processing_nodes += 1
		if node.is_processing_internal():
			internal_processing_nodes += 1
		if node.is_physics_processing_internal():
			internal_physics_processing_nodes += 1
		for child in node.get_children(true):
			if child is Node:
				stack.append(child)
	return {
		"total_nodes": total_nodes,
		"processing_nodes": processing_nodes,
		"physics_processing_nodes": physics_processing_nodes,
		"internal_processing_nodes": internal_processing_nodes,
		"internal_physics_processing_nodes": internal_physics_processing_nodes,
	}


func _resource_activity_summary() -> Dictionary:
	var reload_events: Array[Dictionary] = []
	for entry in _session_logs:
		var event_name := str(entry.get("event", ""))
		if event_name == "session_reload_scene":
			reload_events.append((entry as Dictionary).duplicate(true))
	return {
		"status": "unavailable",
		"reason": "Godot runtime does not expose stable resource import or file-watch activity through the safe live QA bridge",
		"recent_reload_events": reload_events,
	}


func _coerce_positive_frame_count(value: Variant) -> Variant:
	if not value is int and not value is float:
		return null
	var numeric_value := float(value)
	if floor(numeric_value) != numeric_value:
		return null
	var frame_count := int(numeric_value)
	if frame_count <= 0:
		return null
	return frame_count


func _coerce_time_scale(value: Variant) -> Variant:
	if value is bool or not (value is int or value is float):
		return null
	var numeric_value := float(value)
	if not is_finite(numeric_value) or numeric_value <= 0.0 or numeric_value > MAX_TIME_SCALE:
		return null
	return numeric_value


func _coerce_optional_positive_integer(value: Variant) -> Variant:
	if value == null:
		return null
	if value is bool or not (value is int or value is float):
		return null
	var numeric_value := float(value)
	if not is_finite(numeric_value) or floor(numeric_value) != numeric_value:
		return null
	var integer_value := int(numeric_value)
	if integer_value < 1:
		return null
	return integer_value


func _engine_time_settings_snapshot() -> Dictionary:
	return {
		"time_scale": Engine.time_scale,
		"physics_ticks_per_second": Engine.physics_ticks_per_second,
		"max_physics_steps_per_frame": Engine.max_physics_steps_per_frame,
	}


func _reinitialize_recorder() -> void:
	if _recorder != null:
		if _recorder.is_inside_tree():
			var parent := _recorder.get_parent()
			if parent != null:
				parent.remove_child(_recorder)
		_recorder.queue_free()
		_recorder = null
	if not bool(_live_session.get("recorder_enabled", false)):
		return
	var tree := get_tree()
	if tree == null:
		return
	var recorder_options: Dictionary = _live_session.get("recorder", {}) if _live_session.get("recorder", {}) is Dictionary else {}
	_recorder = QaRecorder.new(
		_resolve_scene_root(),
		tree.get_root(),
		QaRuntime.new(_resolve_scene_root(), tree.get_root(), _runtime_errors, _held_actions),
		recorder_options,
	)
	_recorder.name = "GodotQaRecorder"
	tree.get_root().add_child(_recorder)

func append_runtime_error(entry: Dictionary) -> void:
	var normalized := {
		"message": str(entry.get("message", "")),
		"source": str(entry.get("source", "bridge")),
	}
	for key in entry.keys():
		if not key is String:
			continue
		var key_name := str(key)
		if key_name in ["message", "source"]:
			continue
		normalized[key_name] = _json_safe_value(entry.get(key))
	push_error("[godot-qa] %s" % str(normalized.get("message", "")))
	_runtime_errors.append(normalized)
	_record_log("error", "runtime_error", str(normalized.get("message", "")), normalized)


func record_runtime_error(message: String, source := "bridge", extra := {}) -> void:
	var entry := {
		"message": message,
		"source": source,
	}
	if extra is Dictionary:
		for key in extra.keys():
			if key is String:
				entry[str(key)] = extra.get(key)
	append_runtime_error(entry)


func _record_runtime_error(message: String, source := "bridge") -> void:
	record_runtime_error(message, source)


func _json_safe_value(value: Variant) -> Variant:
	if value == null:
		return null
	if value is bool or value is int or value is float or value is String:
		return value
	if value is Array:
		var normalized_array: Array = []
		for item in value:
			normalized_array.append(_json_safe_value(item))
		return normalized_array
	if value is Dictionary:
		var normalized_dict := {}
		for key in value.keys():
			if key is String:
				normalized_dict[str(key)] = _json_safe_value(value.get(key))
		return normalized_dict
	return str(value)

func _record_log(level: String, event_name: String, message: String, data := {}) -> void:
	var entry := {
		"cursor": str(_session_logs.size() + 1),
		"level": level,
		"event": event_name,
		"message": message,
		"data": data if data is Dictionary else {},
	}
	_session_logs.append(entry)

func _read_logs(since: String) -> Dictionary:
	var start_index := 0
	if since != "":
		start_index = maxi(int(since), 0)
	var entries: Array[Dictionary] = []
	for index in range(start_index, _session_logs.size()):
		entries.append((_session_logs[index] as Dictionary).duplicate(true))
	return {
		"entries": entries,
		"next_cursor": str(_session_logs.size()),
	}


func _live_edit_protocol_response(command_name: String, result: Dictionary, request_id: String) -> Dictionary:
	if not bool(result.get("ok", false)):
		var error_payload := (result.get("error", {}) as Dictionary).duplicate(true)
		_record_log(
			"error",
			"runtime_error",
			str(error_payload.get("message", "Live edit command failed")),
			(error_payload.get("data", {}) as Dictionary).duplicate(true),
		)
		return QaProtocol.error(
			str(error_payload.get("type", "runtime_error")),
			str(error_payload.get("message", "Live edit command failed")),
			error_payload.get("data", {}),
			request_id,
		)
	_record_log("info", "live_edit_command", "live edit command completed", {"command": command_name})
	return QaProtocol.ok((result.get("data", {}) as Dictionary).duplicate(true), request_id)


func _is_temporal_assert_command(command_name: String) -> bool:
	return command_name in ["assert.temporal.max_delta_position", "assert.temporal.follows", "assert.temporal.never", "assert.temporal.within_limits", "assert.temporal.distance_changed", "assert.observe_until"]


func _is_visual_assert_command(command_name: String) -> bool:
	return command_name in ["assert.visible_pixels", "assert.not_occluded", "assert.renders_above", "assert.renders_below"]
