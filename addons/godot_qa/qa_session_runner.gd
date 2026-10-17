extends SceneTree

const QaArgs = preload("res://addons/godot_qa/qa_args.gd")
const QaBridge = preload("res://addons/godot_qa/qa_bridge.gd")
const QaLiveSessionHost = preload("res://addons/godot_qa/qa_live_session_host.gd")
const QaProtocol = preload("res://addons/godot_qa/qa_protocol.gd")
const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")

var _bridge: Node = null
var _request := {}
var _scene_instance: Node = null
var _host: Node = null

# QaLiveSessionHost preserves the runner's concurrent request handoff contract:
# _handle_connection(peer)
# _handle_file_request(file_name)


func _initialize() -> void:
	var parsed_args := QaArgs.parse_user_args(OS.get_cmdline_user_args())
	var options: Dictionary = parsed_args.get("options", {})
	var request_path := str(options.get("session_request", ""))
	if request_path == "":
		print(JSON.stringify(QaProtocol.error(
			"missing_session_request",
			"qa_session_runner requires --qa-session-request=<abs-path>",
			{"qa_args": parsed_args},
		)))
		quit(1)
		return

	_request = _read_json_file(request_path)
	if _request.is_empty():
		print(JSON.stringify(QaProtocol.error(
			"request_read_failed",
			"qa_session_runner could not load request.json",
			{"request_path": request_path},
		)))
		quit(1)
		return

	await _run()


func _run() -> void:
	var session_request: Dictionary = _request.get("session", {})
	var artifacts: Dictionary = _request.get("artifacts", {})
	var scene_path := str(session_request.get("scene", ""))
	QaRuntime.clear_provider_registry()
	# A warm session boots WITHOUT a scene (scene == ""): the per-scenario
	# QaScenarioExecutor driven by scenario.run owns the full scene lifecycle
	# (load -> run -> teardown) so every scenario starts from a clean root. A
	# classic live session still boots its scene here.
	if scene_path != "":
		var packed_scene := load(scene_path)
		if packed_scene == null or not packed_scene is PackedScene:
			print(JSON.stringify(QaProtocol.error(
				"scene_load_failed",
				"live session scene could not be loaded",
				{"scene": scene_path},
			)))
			quit(1)
			return

		_scene_instance = packed_scene.instantiate()
		if _scene_instance == null:
			print(JSON.stringify(QaProtocol.error(
				"scene_instantiate_failed",
				"live session scene could not be instantiated",
				{"scene": scene_path},
			)))
			quit(1)
			return

		get_root().add_child(_scene_instance)
	await _apply_requested_viewport(session_request.get("viewport", []))
	await process_frame

	# Reuse the addon's "GodotQaBridge" autoload when present instead of adding a
	# second bridge. Two sibling nodes cannot share a name: a freshly-added
	# `GodotQaBridge` gets auto-renamed (e.g. "@Node@8") when the autoload already
	# occupies that name, so the live-session state (_live_session and the
	# per-scenario project_args set by apply_scenario_options) would live on a
	# node that neither the scenario executor nor the game can find by name. The
	# executor's _adopt_bridge_runtime_errors() and QaArgs.effective_user_args()
	# both resolve the bridge via get_node_or_null("GodotQaBridge") -> the
	# autoload, so the live-session bridge MUST be that same node or per-scenario
	# project_args never reach the game in a warm session.
	var existing_bridge := get_root().get_node_or_null("GodotQaBridge")
	if existing_bridge != null and existing_bridge.has_method("configure_live_session"):
		_bridge = existing_bridge
	else:
		_bridge = QaBridge.new()
		_bridge.name = "GodotQaBridge"
		get_root().add_child(_bridge)
		await process_frame
	if _bridge.has_method("configure_live_session"):
		_bridge.call(
			"configure_live_session",
			{
				"session_id": str(session_request.get("id", "")),
				"scene": scene_path,
				"viewport": session_request.get("viewport", []),
				"started_at": str(session_request.get("started_at", "")),
				"artifacts_root": str(artifacts.get("root", "")),
				"snapshots_dir": str(artifacts.get("snapshots_dir", "")),
				"screenshots_dir": str(artifacts.get("screenshots_dir", "")),
				"audio_dir": str(artifacts.get("audio_dir", "")),
				"recorder_enabled": bool(session_request.get("recorder_enabled", false)),
				"recorder": session_request.get("recorder", {}) if session_request.get("recorder", {}) is Dictionary else {},
			},
		)
	_host = QaLiveSessionHost.new()
	_host.name = "GodotQaLiveSessionHost"
	get_root().add_child(_host)
	var port := int(session_request.get("port", 0))
	var host_result: Dictionary = _host.configure(_bridge, port, str(artifacts.get("root", "")), true)
	if not bool(host_result.get("ok", false)):
		print(JSON.stringify(host_result))
		var host_error: Dictionary = host_result.get("error", {}) if host_result.get("error", {}) is Dictionary else {}
		var host_data: Dictionary = host_error.get("data", {}) if host_error.get("data", {}) is Dictionary else {}
		if not bool(host_data.get("file_transport", false)):
			QaRuntime.clear_provider_registry()
			quit(1)
			return

	await _host.serve()
	QaRuntime.clear_provider_registry()
	quit(0)


func _apply_requested_viewport(viewport_values: Variant) -> void:
	if not viewport_values is Array or viewport_values.size() < 2:
		return
	var width := int(viewport_values[0])
	var height := int(viewport_values[1])
	if width <= 0 or height <= 0:
		return

	var root_window := get_root()
	if root_window.has_method("set_size_2d_override"):
		root_window.call("set_size_2d_override", Vector2i(width, height))
	if root_window.has_method("set_size_2d_override_stretch"):
		root_window.call("set_size_2d_override_stretch", true)
	root_window.min_size = Vector2i(width, height)
	root_window.size = Vector2i(width, height)
	root_window.content_scale_size = Vector2i(width, height)
	await process_frame
	# The first live frame must reach the window. A draw without buffer swap can
	# leave the launch splash visible until a later input forces another present.
	RenderingServer.force_draw(true)
	await process_frame


func _read_json_file(path: String) -> Dictionary:
	var file := FileAccess.open(path, FileAccess.READ)
	if file == null:
		return {}
	var parser := JSON.new()
	var parse_error := parser.parse(file.get_as_text())
	if parse_error != OK or not parser.data is Dictionary:
		return {}
	return parser.data
