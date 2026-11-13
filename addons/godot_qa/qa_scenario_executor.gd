extends RefCounted

const QaScenarioExecutorOperations = preload("res://addons/godot_qa/qa_scenario_executor_operations.gd")
const QaScenarioExecutorProfile = preload("res://addons/godot_qa/qa_scenario_executor_profile.gd")
const QaScenarioExecutorArtifacts = preload("res://addons/godot_qa/qa_scenario_executor_artifacts.gd")
const QaScenarioExecutorManifest = preload("res://addons/godot_qa/qa_scenario_executor_manifest.gd")
const QaAssetTrialCollector = preload("res://addons/godot_qa/qa_asset_trial_collector.gd")
const QaArgs = preload("res://addons/godot_qa/qa_args.gd")
const QaAssertionProof = preload("res://addons/godot_qa/qa_assertion_proof.gd")
const QaBridge = preload("res://addons/godot_qa/qa_bridge.gd")
const QaProfiler = preload("res://addons/godot_qa/qa_profiler.gd")
const QaSoak = preload("res://addons/godot_qa/qa_soak.gd")
const QaProtocol = preload("res://addons/godot_qa/qa_protocol.gd")
const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")
const QaSnapshot = preload("res://addons/godot_qa/qa_snapshot.gd")
const QA_RUNTIME_SCRIPT_PATH := "res://addons/godot_qa/qa_runtime.gd"
const SUPPORTED_STEP_TYPES: Array[String] = [
	"wait_frames",
	"wait_until",
	"snapshot",
	"tap_action",
	"hold_action",
	"release_action",
	"input_key",
	"input_key_hold",
	"input_key_release",
	"input_joy_button",
	"input_joy_motion",
	"drive_policy",
	"click",
	"teleport_node",
	"fixture_teleport",
	"spawn_scene",
	"set_property",
	"joy_connect",
	"joy_disconnect",
	"call_method",
	"fixture_call",
	"node_inspect",
	"record_state",
	"physics_overlay",
	"audio_capture",
	"audio_export",
	"assert",
]
const AUDIO_EXPORT_MAX_BYTES := 10485760
const MAX_PHYSICS_OVERLAY_EVIDENCE := 128
const SUPPORTED_ASSERTION_TYPES: Array[String] = ["exists", "visible", "hidden", "focused", "no_errors"]
const EXPANDED_ASSERTION_TYPES: Array[String] = [
	"exists",
	"not_exists",
	"visible",
	"hidden",
	"enabled",
	"disabled",
	"focused",
	"text_equals",
	"text_contains",
	"inside_viewport",
	"no_overlap",
	"distance",
	"distance_between",
	"distance_changed",
	"observe_until",
	"state_matches",
	"surface_probe",
	"surface_matches",
	"visible_pixels",
	"not_occluded",
	"renders_above",
	"renders_below",
	"max_delta_position",
	"follows",
	"never",
	"within_limits",
	"trend",
	"frame_time",
	"no_errors",
]

var _asset_trial: RefCounted = null
var _assertion_proof: RefCounted = null
var _active_assertion_source_id := ""
var _scenario: Dictionary = {}
var _request_run: Dictionary = {}
var _check: Dictionary = {}
var _artifacts: Dictionary = {}
var _failures: Array[Dictionary] = []
var _runtime_errors: Array = []
var _snapshots: Array[String] = []
var _snapshot_artifacts: Dictionary = {}
var _node_inspections: Dictionary = {}
var _state_records: Dictionary = {}
var _physics_overlays: Dictionary = {}
var _policy_drives: Dictionary = {}
var _audio_artifacts: Dictionary = {}
var _scene_instance: Node = null
var _runtime: RefCounted = null
var _bridge: Node = null
var _held_actions := {}
var _held_keys := {}
var _last_snapshot_name := ""
var _last_snapshot_artifacts: Dictionary = {}
var _started_at_msec := 0
var _mode := "scenario"
var _check_data: Dictionary = {}
var _profile_enabled := false
var _profile_max_frame_samples := 0
var _profile_last_frame_usec := 0
var _profile_frame_times_ms: Array = []
var _profile_frame_samples: Array[Dictionary] = []
var _profile_monitor_samples: Array[Dictionary] = []
var _profile_unsupported_monitors: Array[String] = []
var _profile_step_timings: Array[Dictionary] = []
var _profile_top_script_methods := 10
var _profile_next_step_index := 0
var _profile_next_assertion_index := 0
var _profile_summary: Dictionary = {}
var _soak: RefCounted = null
var _original_max_fps := 0
var _original_temporal_fixed_fps: Variant = null
var _original_time_scale := 1.0
var _original_physics_ticks_per_second := 0
var _original_max_physics_steps_per_frame := 0
var _engine_settings_captured := false
var _scenario_temporal_sampling: Dictionary = {}
var _time_control_evidence: Dictionary = {}
# Frame budget from run.frame_budget in request.json: the CLI derives it from the
# declared scenario timeout x forced fixed-fps, and the engine enforces it here
# because frames are load-independent while wall clocks are not. 0 = disabled.
var _frame_budget_frames := 0
var _frame_budget_start := 0
var _frame_budget_info: Dictionary = {}
# Always-on coarse phase timing (ms floats), independent of the profile gate.
# Emitted as the scenario result's "timing" object so the CLI can attribute
# per-scenario cost (scene load vs steps vs teardown) on every run.
var _timing: Dictionary = {}

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


var _manifest_helper: RefCounted = QaScenarioExecutorManifest.new(self)
var _artifacts_helper: RefCounted = QaScenarioExecutorArtifacts.new(self)
var _profile_helper: RefCounted = QaScenarioExecutorProfile.new(self)
var _operations_helper: RefCounted = QaScenarioExecutorOperations.new(self)
var _tree: SceneTree = null


func _init(tree: SceneTree = null) -> void:
	_tree = tree


# Run a scenario request (already written to disk as request.json) entirely in
# the host SceneTree. Shared by the cold one-shot (qa_runner.gd) and the warm
# in-session handler (qa_bridge.gd). Returns a structured outcome; callers
# decide how to surface it (stdout for cold, protocol response for warm).
#   { status: "request_error", error: <dict>, exit_code: 1 }
#   { status: "finalized", write_ok: bool, result_path: String, payload: {}, exit_code: int }
func run_scenario(request_path: String) -> Dictionary:
	_mode = "scenario"
	_started_at_msec = Time.get_ticks_msec()
	return await _run_scenario(request_path)


# Run a manifest/scene check request. Retains the legacy stdout-printing
# behaviour and returns the process exit code (cold one-shot only).
func run_check(request_path: String) -> int:
	_mode = "check"
	_started_at_msec = Time.get_ticks_msec()
	return await _run_check(request_path)


func _request_error(kind: String, message: String, data: Dictionary) -> Dictionary:
	return {
		"status": "request_error",
		"error": QaProtocol.error(kind, message, data),
		"exit_code": 1,
	}


func _finalize_scenario_outcome() -> Dictionary:
	var finalized: Dictionary = await _finalize_result_payload(true)
	var write_ok := bool(finalized.get("write_ok", true))
	var exit_code := 1
	if write_ok and _failures.is_empty():
		exit_code = 0
	return {
		"status": "finalized",
		"write_ok": write_ok,
		"result_path": finalized.get("result_path", ""),
		"payload": finalized.get("payload", {}),
		"exit_code": exit_code,
	}


func _run_scenario(request_path: String) -> Dictionary:
	var request = _read_json_file(request_path)
	if request == null:
		return _request_error(
			"request_read_failed",
			"qa_runner could not load request.json",
			{"request_path": request_path},
		)

	_asset_trial = QaAssetTrialCollector.new(request.get("asset_trial", {}) if request.get("asset_trial", {}) is Dictionary else {})
	_scenario = request.get("scenario", {})
	_request_run = request.get("run", {})
	_artifacts = request.get("artifacts", {})
	if not _scenario is Dictionary or not _artifacts is Dictionary:
		return _request_error(
			"request_invalid",
			"request.json must contain scenario and artifacts objects",
			{"request_path": request_path},
		)
	# This nonce-bound marker proves that the runtime reached the scenario
	# executor. A later display-looking error is no longer a startup failure.
	_append_jsonl(str(_artifacts.get("progress", _artifacts.get("progress_path", ""))), {
		"event": "runtime_started",
		"run_id": str(_request_run.get("id", "")),
		"nonce": str(_request_run.get("nonce", "")),
	})
	_assertion_proof = QaAssertionProof.new()
	var proof_error: Dictionary = _assertion_proof.initialize(request)
	if not proof_error.is_empty():
		_failures.append(_failure("assertion_proof_invalid", "assertion proof does not match runtime request", proof_error))
		return await _finalize_scenario_outcome()

	# Start counting before scene load so the budget matches the old wall-clock
	# semantics (the process timeout also covered scene boot).
	_init_frame_budget()
	_initialize_profile(request)
	_capture_engine_settings()
	_apply_time_control(_scenario.get("time_control", {}), "scenario")
	_scenario_temporal_sampling = _scenario_temporal_sampling_request()
	var sampling_error := _apply_temporal_sampling(_scenario_temporal_sampling, "scenario")
	if not sampling_error.is_empty():
		_failures.append(sampling_error)
		return await _finalize_scenario_outcome()
	_adopt_bridge_runtime_errors()
	# Expose this scenario's project_args/options to the game via the bridge
	# BEFORE the scene is instantiated, so the game re-seeds RNG / re-reads
	# fixtures in _ready() through QaArgs.effective_user_args(). In cold these
	# equal the cmdline, so behaviour is unchanged; in a warm session they are
	# the per-scenario args.
	if _bridge != null and _bridge.has_method("apply_scenario_options"):
		_bridge.call("apply_scenario_options", _scenario)
	var scene_path := str(_scenario.get("scene", ""))
	QaRuntime.clear_provider_registry()
	var scene_load_started := _profile_now_usec()
	var scene_load_timing_started := Time.get_ticks_usec()
	var scene_failure_count := _failures.size()
	if not await _load_scene_instance(scene_path):
		_record_profile_timing("scene_load", scene_load_started, scene_failure_count, {"target": scene_path})
		_mark_timing("scene_load_ms", scene_load_timing_started)
		return await _finalize_scenario_outcome()
	_record_profile_timing("scene_load", scene_load_started, scene_failure_count, {"target": scene_path})
	_mark_timing("scene_load_ms", scene_load_timing_started)
	var viewport_started := _profile_now_usec()
	await _apply_requested_viewport()
	_record_profile_timing("viewport_setup", viewport_started, _failures.size(), {})
	var initial_frame_started := _profile_now_usec()
	var first_frame_timing_started := Time.get_ticks_usec()
	await _tree.process_frame
	_sample_profile_frame()
	_record_profile_timing("initial_frame_wait", initial_frame_started, _failures.size(), {"frames": 1})
	_mark_timing("first_frame_ms", first_frame_timing_started)
	var qa_runtime_script := load(QA_RUNTIME_SCRIPT_PATH)
	if qa_runtime_script == null or not qa_runtime_script is GDScript:
		_record_runtime_error("Unable to load QA runtime script %s" % QA_RUNTIME_SCRIPT_PATH)
		_failures.append(_failure(
			"runtime_load_failed",
			"QA runtime script could not be loaded",
			{
				"target": QA_RUNTIME_SCRIPT_PATH,
				"expected": "loadable GDScript runtime helper",
				"actual": "missing_or_invalid",
			},
		))
		return await _finalize_scenario_outcome()
	_runtime = (qa_runtime_script as GDScript).new(_scene_instance, _tree.get_root(), _runtime_errors, _held_actions, _held_keys)
	qa_runtime_script = null

	var steps_timing_started := Time.get_ticks_usec()
	for step in _scenario.get("steps", []):
		if _frame_budget_exceeded():
			_append_frame_budget_failure("steps", _profile_next_step_index)
			_mark_timing("steps_ms", steps_timing_started)
			return await _finalize_scenario_outcome()
		await _run_step(step, _profile_next_step_index)
		_profile_next_step_index += 1
	_mark_timing("steps_ms", steps_timing_started)
	if _scenario.has("soak"):
		_soak = QaSoak.new()
		var soak_dir := str(_artifacts.get("run_root", "")).path_join("soak")
		var soak_start: Dictionary = _soak.start(_scenario.get("soak", {}), _runtime, soak_dir, str(_request_run.get("id", "scenario")), str(_artifacts.get("progress", _artifacts.get("progress_path", ""))), Callable(), {"display_server": DisplayServer.get_name(), "vsync_mode": DisplayServer.window_get_vsync_mode(), "renderer": RenderingServer.get_current_rendering_method(), "rendering_driver": RenderingServer.get_current_rendering_driver_name(), "window_size": [DisplayServer.window_get_size().x, DisplayServer.window_get_size().y], "viewport_size": [_tree.root.get_visible_rect().size.x, _tree.root.get_visible_rect().size.y], "time_scale": Engine.time_scale, "max_fps": Engine.max_fps, "physics_ticks_per_second": Engine.physics_ticks_per_second, "pacing": "realtime", "instrumentation": "scenario_profile+soak", "exclusions": ["warmup", "scenario_setup", "final_assertions", "teardown"], "monitor_source": "cached_godot_performance", "monitor_every_frames": int(_scenario.get("soak", {}).get("monitor_every_frames", 60)), "godot_args": _request_run.get("godot_args", [])})
		if not bool(soak_start.get("ok", false)):
			_failures.append(_failure("soak_start_failed", "could not start soak sampler", soak_start))
			return await _finalize_scenario_outcome()
		while str(_soak.status) == "running":
			await _tree.process_frame
			_sample_profile_frame()
			_soak.sample()
		var soak_result: Dictionary = _soak.summary()
		if not bool(soak_result.get("ok", false)):
			_failures.append(_failure("soak_budget_failed", "soak did not complete within budgets", {"target": str(_scenario.get("name", "")), "expected": "all windows complete within budgets", "actual": soak_result, "artifact": soak_dir.path_join("soak.json")}))

	var asserts_timing_started := Time.get_ticks_usec()
	var final_assertion_index := 0
	for assertion in _scenario.get("assert", []):
		if _frame_budget_exceeded():
			_append_frame_budget_failure("assert", _profile_next_assertion_index)
			_mark_timing("asserts_ms", asserts_timing_started)
			return await _finalize_scenario_outcome()
		await _run_assertion(assertion, _profile_next_assertion_index, "/assert/%d" % final_assertion_index)
		_profile_next_assertion_index += 1
		final_assertion_index += 1
	_mark_timing("asserts_ms", asserts_timing_started)

	# Catch a budget that trips during the last step/assertion: a blown budget
	# must fail deterministically, exactly as the old wall kill would have.
	if _frame_budget_exceeded():
		_append_frame_budget_failure("finalize", -1)
	return await _finalize_scenario_outcome()


func _run_check(request_path: String) -> int:
	var request = _read_json_file(request_path)
	if request == null:
		print(JSON.stringify(QaProtocol.error(
			"request_read_failed",
			"qa_runner could not load request.json",
			{"request_path": request_path},
		)))
		return 1

	_check = request.get("check", {})
	_artifacts = request.get("artifacts", {})
	if not _check is Dictionary or not _artifacts is Dictionary:
		print(JSON.stringify(QaProtocol.error(
			"request_invalid",
			"request.json must contain check and artifacts objects",
			{"request_path": request_path},
		)))
		return 1

	_adopt_bridge_runtime_errors()
	_capture_engine_settings()
	_initialize_profile(request)
	if str(_check.get("execution_mode", "")) == "batch" or _check.has("tests"):
		return await _run_manifest_batch_check()
	_apply_time_control(_check.get("time_control", {}), "check")
	if _check.has("kind") and _check.has("target"):
		await _run_manifest_check()
	else:
		var scene_path := str(_check.get("scene", ""))
		QaRuntime.clear_provider_registry()
		if not await _load_scene_instance(scene_path):
			return await _finalize_result()

		await _tree.process_frame
		RenderingServer.force_draw(false)
		await _tree.process_frame

		var startup_frames := max(int(_check.get("startup_frames", 0)), 0)
		for _frame_index in range(startup_frames):
			await _tree.process_frame

		if _failures.is_empty() and not _runtime_errors.is_empty():
			_failures.append(_failure(
				"runtime_errors_present",
				"runtime errors were recorded during startup",
				{
					"target": scene_path,
					"expected": [],
					"actual": _runtime_errors.duplicate(true),
				},
			))

	return await _finalize_result()


func _run_manifest_batch_check() -> int:
	return await _manifest_helper._run_manifest_batch_check()


func _run_manifest_check() -> void:
	await _manifest_helper._run_manifest_check()


func _apply_requested_viewport() -> void:
	var viewport_request = _scenario.get("viewport", {})
	if not viewport_request is Dictionary:
		return

	var width := int(viewport_request.get("width", 0))
	var height := int(viewport_request.get("height", 0))
	if width <= 0 or height <= 0:
		return

	var root_window := _tree.get_root()
	if root_window.has_method("set_size_2d_override"):
		root_window.call("set_size_2d_override", Vector2i(width, height))
	if root_window.has_method("set_size_2d_override_stretch"):
		root_window.call("set_size_2d_override_stretch", true)
	root_window.min_size = Vector2i(width, height)
	root_window.size = Vector2i(width, height)
	root_window.content_scale_size = Vector2i(width, height)
	await _tree.process_frame
	_sample_profile_frame()
	RenderingServer.force_draw(false)
	await _tree.process_frame
	_sample_profile_frame()


func _run_step(step: Variant, step_index := -1) -> void:
	var started_usec := _progress_now_usec()
	var failure_count := _failures.size()
	var step_kind := _profile_step_kind(step)
	var timing_extra := {"index": step_index, "kind": step_kind}
	var heartbeat_context := _progress_context_for_operation(step, "step")
	var heartbeat_command := ""
	if step is Dictionary:
		var heartbeat_runtime_command := _runtime_command_for_step(step)
		if not heartbeat_runtime_command.is_empty():
			heartbeat_command = str(heartbeat_runtime_command.get("command", ""))
			heartbeat_context = heartbeat_runtime_command.get("progress_context", heartbeat_context)
	var heartbeat_started_at := _progress_started_at()
	_write_scenario_heartbeat("start", "step", step_index, step_kind, heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)
	if not step is Dictionary or step.size() != 1:
		_failures.append(_failure(
			"unsupported_step",
			"step must be an object with exactly one operation",
			{
				"target": "step",
				"expected": "object with exactly one supported operation",
				"actual": step,
			},
		))
		_record_profile_timing("step", started_usec, failure_count, timing_extra)
		_write_scenario_heartbeat("finish", "step", step_index, step_kind, heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)
		return

	if step.has("wait_frames"):
		var frame_count := int(step.get("wait_frames", 0))
		var frames_waited := 0
		for _i in range(max(frame_count, 0)):
			if _frame_budget_exceeded():
				break
			await _tree.process_frame
			_sample_profile_frame()
			frames_waited += 1
		timing_extra["frames"] = frames_waited
		if not _scenario_temporal_sampling.is_empty():
			timing_extra["temporal_sampling"] = _scenario_temporal_sampling.duplicate(true)
		_record_profile_timing("step", started_usec, failure_count, timing_extra)
		_write_scenario_heartbeat("finish", "step", step_index, step_kind, heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)
		return

	if step.has("wait_until"):
		var wait_result: Dictionary = await _run_wait_until(step.get("wait_until", {}))
		if not bool(wait_result.get("ok", false)):
			_failures.append(_failure(
				str(wait_result.get("type", "wait_until_timeout")),
				str(wait_result.get("message", "wait_until predicate did not hold within timeout_frames")),
				wait_result.get("data", {}),
			))
		timing_extra["frames"] = int(wait_result.get("frames", 0))
		_record_profile_timing("step", started_usec, failure_count, timing_extra)
		_write_scenario_heartbeat("finish", "step", step_index, step_kind, heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)
		return

	if step.has("drive_policy"):
		var drive_result: Dictionary = await _run_drive_policy(step.get("drive_policy", {}), step_index)
		if not bool(drive_result.get("ok", false)):
			_failures.append(_failure(
				str(drive_result.get("type", "drive_policy_failed")),
				str(drive_result.get("message", "drive_policy step failed")),
				drive_result.get("data", {}),
			))
		timing_extra["frames"] = int(drive_result.get("frames", 0))
		timing_extra["ticks"] = int(drive_result.get("ticks", 0))
		timing_extra["stop_reason"] = str(drive_result.get("stop_reason", ""))
		_record_profile_timing("step", started_usec, failure_count, timing_extra)
		_write_scenario_heartbeat("finish", "step", step_index, step_kind, heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)
		return

	if step.has("snapshot"):
		var snapshot_name := str(step.get("snapshot", ""))
		if snapshot_name == "":
			_failures.append(_failure(
				"unsupported_step",
				"snapshot step requires a non-empty name",
				{
					"target": "snapshot",
					"expected": "non-empty snapshot name",
					"actual": step,
				},
			))
			_record_profile_timing("step", started_usec, failure_count, timing_extra)
			_write_scenario_heartbeat("finish", "step", step_index, step_kind, heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)
			return
		await _capture_snapshot(snapshot_name)
		timing_extra["snapshot"] = snapshot_name
		_record_profile_timing("step", started_usec, failure_count, timing_extra)
		_write_scenario_heartbeat("finish", "step", step_index, step_kind, heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)
		return

	if step.has("assert"):
		var assertions = step.get("assert", [])
		if not assertions is Array:
			_failures.append(_failure(
				"unsupported_step",
				"assert step requires an assertion list",
				{
					"target": "assert",
					"expected": "list of assertion objects",
					"actual": assertions,
				},
			))
			_record_profile_timing("step", started_usec, failure_count, timing_extra)
			_write_scenario_heartbeat("finish", "step", step_index, step_kind, heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)
			return
		for assertion_index in range(assertions.size()):
			await _run_assertion(assertions[assertion_index], _profile_next_assertion_index, "/steps/%d/assert/%d" % [step_index, assertion_index])
			_profile_next_assertion_index += 1
		timing_extra["assertion_count"] = assertions.size()
		_record_profile_timing("step", started_usec, failure_count, timing_extra)
		_write_scenario_heartbeat("finish", "step", step_index, step_kind, heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)
		return

	if step.has("audio_capture"):
		var audio_capture_params = step.get("audio_capture", {})
		if not audio_capture_params is Dictionary:
			_failures.append(_audio_failure(
				"audio_capture_failed",
				"audio_capture step requires an object",
				"audio_capture",
				{
					"expected": "object with name, duration_seconds, and optional bus",
					"actual": audio_capture_params,
				},
			))
		else:
			await _run_audio_capture_step(audio_capture_params)
			timing_extra["audio"] = str((audio_capture_params as Dictionary).get("name", ""))
		_record_profile_timing("step", started_usec, failure_count, timing_extra)
		_write_scenario_heartbeat("finish", "step", step_index, step_kind, "audio_capture", heartbeat_context, heartbeat_started_at, started_usec, failure_count)
		return

	if step.has("audio_export"):
		var audio_export_params = step.get("audio_export", {})
		if not audio_export_params is Dictionary:
			_failures.append(_audio_failure(
				"audio_export_invalid",
				"audio_export step requires an object",
				"audio_export",
				{
					"expected": "object with name, target, method, and optional args",
					"actual": audio_export_params,
				},
			))
		else:
			await _run_audio_export_step(audio_export_params)
			timing_extra["audio"] = str((audio_export_params as Dictionary).get("name", ""))
		_record_profile_timing("step", started_usec, failure_count, timing_extra)
		_write_scenario_heartbeat("finish", "step", step_index, step_kind, "audio_export", heartbeat_context, heartbeat_started_at, started_usec, failure_count)
		return

	var runtime_command := _runtime_command_for_step(step)
	if not runtime_command.is_empty():
		timing_extra["command"] = str(runtime_command.get("command", ""))
		heartbeat_context = runtime_command.get("progress_context", heartbeat_context)
		var runtime_started_usec := _profile_now_usec()
		var runtime_result: Dictionary = await _runtime.execute(
			str(runtime_command.get("command", "")),
			runtime_command.get("params", {}),
		)
		_record_profile_timing(
			"runtime_request",
			runtime_started_usec,
			failure_count,
			{
				"index": step_index,
				"kind": "runtime_request",
				"command": str(runtime_command.get("command", "")),
			},
		)
		if not bool(runtime_result.get("ok", false)):
			_failures.append(_runtime_error_to_failure(runtime_result.get("error", {})))
		elif str(runtime_command.get("command", "")) == "node.inspect":
			var inspect_params: Dictionary = runtime_command.get("params", {})
			_record_node_inspection(step_index, str(inspect_params.get("target", "")), runtime_result)
		elif str(runtime_command.get("command", "")) == "state.record":
			var record_params: Dictionary = runtime_command.get("params", {})
			_record_state_record(step_index, str(record_params.get("name", "")), runtime_result)
		elif str(runtime_command.get("command", "")) == "physics.overlay.collect":
			var overlay_params: Dictionary = runtime_command.get("params", {})
			_record_physics_overlay(step_index, step, overlay_params, runtime_result)
		_record_profile_timing("step", started_usec, failure_count, timing_extra)
		_write_scenario_heartbeat("finish", "step", step_index, step_kind, str(runtime_command.get("command", "")), heartbeat_context, heartbeat_started_at, started_usec, failure_count)
		return

	_failures.append(_failure(
		"unsupported_step",
		"M03 runner does not support this step yet",
		{
			"target": _operation_name(step, "step"),
			"expected": SUPPORTED_STEP_TYPES,
			"actual": step,
		},
	))
	_record_profile_timing("step", started_usec, failure_count, timing_extra)
	_write_scenario_heartbeat("finish", "step", step_index, step_kind, heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)


func _init_frame_budget() -> void:
	_frame_budget_frames = 0
	_frame_budget_info = {}
	var budget_variant: Variant = _request_run.get("frame_budget", {})
	if not budget_variant is Dictionary:
		return
	var budget: Dictionary = budget_variant
	var frames := int(budget.get("frames", 0))
	if frames <= 0:
		return
	_frame_budget_frames = frames
	_frame_budget_info = budget.duplicate(true)
	_frame_budget_start = Engine.get_process_frames()


func _frame_budget_exceeded() -> bool:
	if _frame_budget_frames <= 0:
		return false
	return Engine.get_process_frames() - _frame_budget_start > _frame_budget_frames


# The single authoritative frame-budget failure, appended only at step/assert
# boundaries (frame-consuming loops just stop early and defer to this).
func _append_frame_budget_failure(phase: String, active_index: int) -> void:
	_failures.append(_failure(
		"scenario_frame_budget_exceeded",
		"scenario exceeded its frame budget (declared timeout at forced frame pacing)",
		{
			"expected": "scenario completes within %d frames" % _frame_budget_frames,
			"actual": {
				"budget_frames": _frame_budget_frames,
				"consumed_frames": Engine.get_process_frames() - _frame_budget_start,
				"phase": phase,
				"active_index": active_index,
				"derived_from": _frame_budget_info.duplicate(true),
			},
		},
	))


func _run_wait_until(params: Variant) -> Dictionary:
	return await _operations_helper._run_wait_until(params)


func _run_drive_policy(params: Variant, step_index: int) -> Dictionary:
	return await _operations_helper._run_drive_policy(params, step_index)


func _run_assertion(assertion: Variant, assertion_index := -1, runtime_path := "") -> void:
	await _operations_helper._run_assertion(assertion, assertion_index, runtime_path)


func _runtime_command_for_step(step: Dictionary) -> Dictionary:
	return _operations_helper._runtime_command_for_step(step)


func _operation_name(entry: Dictionary, fallback: String) -> String:
	return _operations_helper._operation_name(entry, fallback)


func _runtime_error_to_failure(error_data: Variant) -> Dictionary:
	if error_data is Dictionary:
		return _failure(
			str(error_data.get("type", "runtime_error")),
			str(error_data.get("message", "Runtime command failed")),
			error_data.get("data", {}),
		)
	return _failure(
		"runtime_error",
		"Runtime command failed",
		{
			"actual": error_data,
		},
	)


func _initialize_profile(request: Dictionary) -> void:
	_profile_helper._initialize_profile(request)


func _profile_now_usec() -> int:
	return _profile_helper._profile_now_usec()


func _mark_timing(key: String, started_usec: int) -> void:
	_profile_helper._mark_timing(key, started_usec)


func _record_profile_timing(label: String, started_usec: int, failure_count: int, extra: Dictionary) -> void:
	_profile_helper._record_profile_timing(label, started_usec, failure_count, extra)


func _profile_step_kind(step: Variant) -> String:
	return _profile_helper._profile_step_kind(step)


func _record_node_inspection(step_index: int, target: String, result: Dictionary) -> void:
	_artifacts_helper._record_node_inspection(step_index, target, result)


func _record_state_record(step_index: int, name: String, result: Dictionary) -> void:
	_artifacts_helper._record_state_record(step_index, name, result)


func _physics_overlay_step_params(raw_params: Variant) -> Dictionary:
	return _artifacts_helper._physics_overlay_step_params(raw_params)


func _record_physics_overlay(step_index: int, step: Dictionary, params: Dictionary, result: Dictionary) -> void:
	_artifacts_helper._record_physics_overlay(step_index, step, params, result)


func _sample_profile_frame() -> void:
	_profile_helper._sample_profile_frame()


func _profile_payload() -> Dictionary:
	return _profile_helper._profile_payload()


func _capture_snapshot(snapshot_name: String) -> void:
	await _artifacts_helper._capture_snapshot(snapshot_name)


func _run_audio_capture_step(params: Dictionary) -> void:
	await _artifacts_helper._run_audio_capture_step(params)


func _run_audio_export_step(params: Dictionary) -> void:
	await _artifacts_helper._run_audio_export_step(params)


func _audio_failure(failure_type: String, message: String, command: String, extra: Dictionary) -> Dictionary:
	return _artifacts_helper._audio_failure(failure_type, message, command, extra)


func _failure(failure_type: String, message: String, extra := {}) -> Dictionary:
	var failure := {
		"type": failure_type,
		"message": message,
		"severity": "error",
	}
	if not _last_snapshot_artifacts.is_empty():
		failure["artifacts"] = _last_snapshot_artifacts.duplicate(true)
	else:
		var run_root := _artifact_run_root()
		if run_root != "":
			failure["artifacts"] = {"run_root": run_root}
	if extra is Dictionary:
		for key in extra.keys():
			failure[key] = extra[key]
	return failure


func _record_runtime_error(message: String, source := "runner") -> void:
	push_error("[godot-qa] %s" % message)
	_runtime_errors.append(
		{
			"message": message,
			"source": source,
		}
	)


func _adopt_bridge_runtime_errors() -> void:
	var bridge: Node = _tree.get_root().get_node_or_null("GodotQaBridge")
	if bridge == null:
		bridge = QaBridge.new()
		bridge.name = "GodotQaBridge"
		_tree.get_root().add_child(bridge)
	_bridge = bridge
	if bridge == null:
		return
	if bridge.has_method("clear_runtime_errors"):
		bridge.call("clear_runtime_errors")
	if bridge.has_method("get_runtime_errors"):
		var shared_errors = bridge.call("get_runtime_errors")
		if shared_errors is Array:
			_runtime_errors = shared_errors


func _load_scene_instance(scene_path: String) -> bool:
	QaRuntime.clear_provider_registry()
	var packed_scene := load(scene_path)
	if packed_scene == null or not packed_scene is PackedScene:
		_record_runtime_error("Unable to load scene %s" % scene_path)
		_failures.append(_failure(
			"scene_load_failed",
			"scene could not be loaded",
			{
				"target": scene_path,
				"expected": "loadable PackedScene",
				"actual": "missing_or_invalid",
			},
		))
		return false

	_scene_instance = packed_scene.instantiate()
	packed_scene = null
	if _scene_instance == null:
		_record_runtime_error("PackedScene.instantiate() returned null for %s" % scene_path)
		_failures.append(_failure(
			"scene_instantiate_failed",
			"scene could not be instantiated",
			{
				"target": scene_path,
				"expected": "instantiated scene tree",
				"actual": "null",
			},
		))
		return false

	_tree.get_root().add_child(_scene_instance)
	return true


func _result_payload() -> Dictionary:
	return _artifacts_helper._result_payload()


func _finalize_result() -> int:
	var finalized: Dictionary = await _finalize_result_payload(true)
	if not bool(finalized.get("write_ok", true)):
		print(JSON.stringify(QaProtocol.error(
			"result_write_failed",
			"qa_runner could not write result.json",
			{"result_path": finalized.get("result_path", "")},
		)))
		return 1

	print(JSON.stringify(finalized.get("payload", {})))
	return 0 if _failures.is_empty() else 1


func _finalize_result_payload(write_result: bool) -> Dictionary:
	if not _held_keys.is_empty():
		var held_keys: Array = _held_keys.keys()
		held_keys.sort()
		_failures.append(_failure(
			"runtime_contamination_detected",
			"keyboard keys still held at scenario finalization: %s" % ", ".join(held_keys),
			{
				"command": "input.key_release",
				"target": str(_scenario.get("name", _check.get("id", "check"))),
				"expected": [],
				"actual": held_keys,
			},
		))
	if _runtime != null:
		var release_started_usec := _profile_now_usec()
		var failure_count := _failures.size()
		var key_release_failures: Array = await _runtime.release_held_keys()
		for key_failure in key_release_failures:
			_failures.append(_runtime_error_to_failure(key_failure))
		var release_failures: Array = await _runtime.release_held_actions()
		for release_failure in release_failures:
			_failures.append(_runtime_error_to_failure(release_failure))
		# Zero any joypad axes injected this scenario so a held trigger/stick
		# cannot leak into the next warm scenario sharing this process. No-op for
		# scenarios that touched no axes, so cold result.json is unchanged.
		var axis_release_failures: Array = await _runtime.release_held_axes()
		for axis_failure in axis_release_failures:
			_failures.append(_runtime_error_to_failure(axis_failure))
		# Release any joypad buttons still pressed by injected events (a
		# drive_policy hold that never released) for the same reason.
		var button_release_failures: Array = await _runtime.release_held_buttons()
		for button_failure in button_release_failures:
			_failures.append(_runtime_error_to_failure(button_failure))
		# Lift any screen touches still held by injected events so a finger cannot
		# leak into the next warm scenario sharing this process.
		var touch_release_failures: Array = await _runtime.release_held_touches()
		for touch_failure in touch_release_failures:
			_failures.append(_runtime_error_to_failure(touch_failure))
		# Fire a matching disconnect for any joypads faked via joy_connect and
		# clear the QA tracking set. What would otherwise leak into the next warm
		# scenario is not the engine roster (never mutated on this engine) but the
		# announced-connected state a long-lived game listener saw with no matching
		# disconnect, plus the tracking set that drives joy_disconnect's real-pad
		# guard and the effective_connected_joypads() roster shim.
		var joypad_release_failures: Array = await _runtime.release_faked_joypads()
		for joypad_failure in joypad_release_failures:
			_failures.append(_runtime_error_to_failure(joypad_failure))
		_record_profile_timing("held_action_release", release_started_usec, failure_count, {
			"kind": "input",
			"release_count": release_failures.size(),
		})
	if not _held_actions.is_empty():
		_failures.append(_failure(
			"runtime_contamination_detected",
			"held input actions remained after runtime cleanup",
			{
				"target": str(_check.get("id", _check.get("scene", "check"))),
				"expected": {},
				"actual": _held_actions.keys(),
			},
		))
	var scene_teardown_started := _profile_now_usec()
	var teardown_timing_started := Time.get_ticks_usec()
	var scene_teardown_failure_count := _failures.size()
	await _teardown_runtime_scene()
	_record_profile_timing("scene_teardown", scene_teardown_started, scene_teardown_failure_count, {})
	_mark_timing("teardown_ms", teardown_timing_started)
	var engine_restore_started := _profile_now_usec()
	var engine_restore_failure_count := _failures.size()
	_restore_engine_settings()
	_record_profile_timing("engine_settings_restore", engine_restore_started, engine_restore_failure_count, {})

	var result_path := _artifact_result_path()
	var payload := _result_payload()
	if write_result and result_path != "":
		var write_error := _write_json_file(result_path, payload)
		if write_error != OK:
			return {
				"write_ok": false,
				"result_path": result_path,
				"payload": payload,
			}
	return {
		"write_ok": true,
		"result_path": result_path,
		"payload": payload,
	}


func _write_check_progress_event(event: String, check: Dictionary, artifacts: Dictionary, batch_artifacts: Dictionary, started_at: String, elapsed_ms, ok, failure_count) -> void:
	_profile_helper._write_check_progress_event(event, check, artifacts, batch_artifacts, started_at, elapsed_ms, ok, failure_count)


func _write_scenario_heartbeat(event: String, operation: String, index: int, kind: String, command: String, context: Dictionary, started_at: String, started_usec: int, failure_count: int) -> void:
	_profile_helper._write_scenario_heartbeat(event, operation, index, kind, command, context, started_at, started_usec, failure_count)


func _progress_now_usec() -> int:
	return _profile_helper._progress_now_usec()


func _progress_context_for_operation(entry: Variant, fallback: String) -> Dictionary:
	return _profile_helper._progress_context_for_operation(entry, fallback)


func _safe_progress_context(params: Variant) -> Dictionary:
	return _profile_helper._safe_progress_context(params)


func _append_jsonl(path: String, payload: Dictionary) -> void:
	_profile_helper._append_jsonl(path, payload)


func _progress_started_at() -> String:
	return _profile_helper._progress_started_at()


func _clear_bridge_runtime_errors() -> void:
	if _bridge != null and _bridge.has_method("clear_runtime_errors"):
		_bridge.call("clear_runtime_errors")


func _reset_check_runtime_state() -> void:
	_failures.clear()
	_runtime_errors.clear()
	_check_data.clear()
	_time_control_evidence.clear()
	_last_snapshot_artifacts.clear()
	_last_snapshot_name = ""
	_audio_artifacts.clear()
	_runtime = null
	_scene_instance = null
	_held_actions.clear()
	_held_keys.clear()


func _artifact_run_root() -> String:
	return str(_artifacts.get("run_root", _artifacts.get("root", "")))


func _artifact_result_path() -> String:
	return str(_artifacts.get("result_path", _artifacts.get("result", "")))


func _capture_engine_settings() -> void:
	if _engine_settings_captured:
		return
	_original_max_fps = Engine.max_fps
	_original_temporal_fixed_fps = _tree.root.get_meta("_godot_qa_temporal_fixed_fps") if _tree.root.has_meta("_godot_qa_temporal_fixed_fps") else null
	var pacing: Variant = _request_run.get("frame_pacing", _check.get("frame_pacing", {}))
	_tree.root.set_meta("_godot_qa_temporal_fixed_fps", int(pacing.get("fixed_fps", 0)) if pacing is Dictionary else 0)
	_original_time_scale = Engine.time_scale
	_original_physics_ticks_per_second = Engine.physics_ticks_per_second
	_original_max_physics_steps_per_frame = Engine.max_physics_steps_per_frame
	_engine_settings_captured = true


func _engine_time_settings_snapshot() -> Dictionary:
	return {
		"time_scale": Engine.time_scale,
		"physics_ticks_per_second": Engine.physics_ticks_per_second,
		"max_physics_steps_per_frame": Engine.max_physics_steps_per_frame,
	}


func _original_engine_time_settings_snapshot() -> Dictionary:
	return {
		"time_scale": _original_time_scale,
		"physics_ticks_per_second": _original_physics_ticks_per_second,
		"max_physics_steps_per_frame": _original_max_physics_steps_per_frame,
	}


func _has_time_control_evidence() -> bool:
	return not _time_control_evidence.is_empty()


func _apply_time_control(raw_time_control: Variant, target: String) -> void:
	if not raw_time_control is Dictionary or (raw_time_control as Dictionary).is_empty():
		return
	var time_control := (raw_time_control as Dictionary).duplicate(true)
	var warnings: Array = []
	if time_control.has("time_scale"):
		Engine.time_scale = float(time_control.get("time_scale"))
	if time_control.has("physics_ticks_per_second"):
		Engine.physics_ticks_per_second = int(time_control.get("physics_ticks_per_second"))
	if time_control.has("max_physics_steps_per_frame"):
		Engine.max_physics_steps_per_frame = int(time_control.get("max_physics_steps_per_frame"))
	if time_control.has("time_scale") and (not time_control.has("physics_ticks_per_second") or not time_control.has("max_physics_steps_per_frame")):
		warnings.append({
			"type": "time_control_physics_pairing_missing",
			"message": "time_scale acceleration was requested without both paired physics overrides",
			"target": target,
		})
	_time_control_evidence = {
		"target": target,
		"original": _original_engine_time_settings_snapshot(),
		"requested": time_control,
		"effective": _engine_time_settings_snapshot(),
		"restored": {},
		"restored_matches_original": false,
		"restoration_status": "pending",
		"warnings": warnings,
	}


func _restore_engine_settings() -> void:
	if not _engine_settings_captured:
		return
	Engine.max_fps = _original_max_fps
	if _original_temporal_fixed_fps == null:
		_tree.root.remove_meta("_godot_qa_temporal_fixed_fps")
	else:
		_tree.root.set_meta("_godot_qa_temporal_fixed_fps", _original_temporal_fixed_fps)
	Engine.time_scale = _original_time_scale
	Engine.physics_ticks_per_second = _original_physics_ticks_per_second
	Engine.max_physics_steps_per_frame = _original_max_physics_steps_per_frame
	if _has_time_control_evidence():
		var restored := _engine_time_settings_snapshot()
		var original := _original_engine_time_settings_snapshot()
		var restored_matches_original := restored == original
		_time_control_evidence["restored"] = restored
		_time_control_evidence["restored_matches_original"] = restored_matches_original
		_time_control_evidence["restoration_status"] = "restored" if restored_matches_original else "mismatch"


func _scenario_temporal_sampling_request() -> Dictionary:
	var determinism = _scenario.get("determinism", {})
	if not determinism is Dictionary:
		return {}
	var sampling = (determinism as Dictionary).get("temporal_sampling", {})
	if sampling is Dictionary:
		return (sampling as Dictionary).duplicate(true)
	return {}


func _apply_temporal_sampling(sampling: Dictionary, target: String) -> Dictionary:
	if sampling.is_empty():
		return {}
	var mode := str(sampling.get("mode", ""))
	if mode == "fixed_delta":
		var seconds := float(sampling.get("fixed_delta_seconds", 0.0))
		if seconds <= 0.0 or not is_finite(seconds):
			return _failure("invalid_sampling", "fixed_delta temporal sampling requires a positive duration", {"target": target, "expected": "fixed_delta_seconds > 0", "actual": sampling.duplicate(true)})
		var fps := max(int(round(1.0 / seconds)), 1)
		Engine.physics_ticks_per_second = fps
		Engine.max_fps = fps
		_refresh_time_control_effective()
		return {}
	if mode == "max_frame_duration":
		var max_seconds := float(sampling.get("max_frame_duration_seconds", 0.0))
		if max_seconds <= 0.0 or not is_finite(max_seconds):
			return _failure("invalid_sampling", "max_frame_duration temporal sampling requires a positive duration", {"target": target, "expected": "max_frame_duration_seconds > 0", "actual": sampling.duplicate(true)})
		Engine.max_fps = max(int(ceil(1.0 / max_seconds)), 1)
		_refresh_time_control_effective()
		return {}
	return _failure("invalid_sampling", "temporal sampling mode is unsupported", {"target": target, "expected": ["fixed_delta", "max_frame_duration"], "actual": sampling.duplicate(true)})


func _refresh_time_control_effective() -> void:
	if _has_time_control_evidence():
		_time_control_evidence["effective"] = _engine_time_settings_snapshot()


func _teardown_runtime_scene() -> void:
	if _runtime != null and _runtime.has_method("teardown"):
		_runtime.call("teardown")
	_runtime = null
	_held_actions.clear()
	_held_keys.clear()
	QaRuntime.clear_provider_registry()
	if _scene_instance == null:
		return

	var scene_instance := _scene_instance
	_scene_instance = null
	var parent := scene_instance.get_parent()
	if parent != null:
		parent.remove_child(scene_instance)
	scene_instance.free()
	await _tree.process_frame


func _read_json_file(path: String) -> Variant:
	var file := FileAccess.open(path, FileAccess.READ)
	if file == null:
		return null
	var parser := JSON.new()
	var parse_error := parser.parse(file.get_as_text())
	if parse_error != OK:
		return null
	return parser.data


func _write_json_file(path: String, payload: Dictionary) -> int:
	var file := FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		return ERR_CANT_OPEN
	file.store_string(JSON.stringify(payload))
	return OK
