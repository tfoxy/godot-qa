extends RefCounted

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
	_runtime = (qa_runtime_script as GDScript).new(_scene_instance, _tree.get_root(), _runtime_errors, _held_actions)
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
	var tests_value = _check.get("tests", null)
	if not tests_value is Array:
		_failures.append(_failure(
			"batch_check_request_invalid",
			"batch check requests require a tests array",
			{
				"target": "check.tests",
				"expected": "array of manifest-backed check entries",
				"actual": tests_value,
			},
		))
		return await _finalize_result()

	var tests: Array = tests_value
	var validation_failures: Array[Dictionary] = []
	for index in range(tests.size()):
		var entry = tests[index]
		if not entry is Dictionary:
			validation_failures.append(_failure(
				"batch_check_request_invalid",
				"batch check entries must be objects",
				{
					"target": "check.tests[%d]" % index,
					"expected": "object with id, kind, target, and artifacts",
					"actual": entry,
				},
			))
			continue
		var entry_dict: Dictionary = entry
		var test_id := str(entry_dict.get("id", ""))
		var kind := str(entry_dict.get("kind", ""))
		var target := str(entry_dict.get("target", ""))
		var artifacts = entry_dict.get("artifacts", null)
		if test_id == "" or kind == "" or target == "" or not artifacts is Dictionary:
			validation_failures.append(_failure(
				"batch_check_request_invalid",
				"batch check entries require id, kind, target, and artifacts",
				{
					"target": test_id if test_id != "" else "check.tests[%d]" % index,
					"expected": {
						"id": "non-empty string",
						"kind": "scene or script",
						"target": "non-empty string",
						"artifacts": "object",
					},
					"actual": entry_dict.duplicate(true),
				},
			))

	if not validation_failures.is_empty():
		for validation_failure in validation_failures:
			_failures.append(validation_failure)
		return await _finalize_result()

	var batch_artifacts := _artifacts.duplicate(true)
	var batch_started_at := _started_at_msec
	var summary_entries: Array[Dictionary] = []
	var aggregate_failures: Array[Dictionary] = []
	var had_artifact_write_failure := false
	for index in range(tests.size()):
		var entry: Dictionary = tests[index]
		var entry_check: Dictionary = {}
		if entry.get("check", null) is Dictionary:
			entry_check = (entry.get("check") as Dictionary).duplicate(true)
		entry_check["id"] = str(entry.get("id", ""))
		entry_check["kind"] = str(entry.get("kind", ""))
		entry_check["target"] = str(entry.get("target", ""))
		if entry.has("startup_frames"):
			entry_check["startup_frames"] = entry.get("startup_frames")
		if entry.has("time_control"):
			entry_check["time_control"] = entry.get("time_control")

		_reset_check_runtime_state()
		_started_at_msec = Time.get_ticks_msec()
		_check = entry_check
		_artifacts = (entry.get("artifacts") as Dictionary).duplicate(true)
		_initialize_profile(entry)
		var progress_started_at := _progress_started_at()
		var progress_started_msec := Time.get_ticks_msec()
		_write_check_progress_event("start", entry_check, _artifacts, batch_artifacts, progress_started_at, null, null, null)
		_apply_time_control(_check.get("time_control", {}), str(_check.get("id", "")))
		# Snapshot AFTER applying this test's own declared time_control (its
		# expected engine state) so a later mismatch means the test itself
		# mutated the shared Engine time/physics settings — not the declared
		# mechanism — and would otherwise silently leak into the next test
		# sharing this batch process. Covers the same four settings
		# _capture_engine_settings/_restore_engine_settings treat as
		# batch-global state.
		var expected_time_scale := Engine.time_scale
		var expected_physics_ticks := Engine.physics_ticks_per_second
		var expected_max_fps := Engine.max_fps
		var expected_max_physics_steps := Engine.max_physics_steps_per_frame
		await _run_manifest_check()
		if (
			Engine.time_scale != expected_time_scale
			or Engine.physics_ticks_per_second != expected_physics_ticks
			or Engine.max_fps != expected_max_fps
			or Engine.max_physics_steps_per_frame != expected_max_physics_steps
		):
			_failures.append(_failure(
				"runtime_contamination_detected",
				"Engine time/physics settings changed during the test and were not restored",
				{
					"target": str(_check.get("id", "")),
					"expected": {
						"time_scale": expected_time_scale,
						"physics_ticks_per_second": expected_physics_ticks,
						"max_fps": expected_max_fps,
						"max_physics_steps_per_frame": expected_max_physics_steps,
					},
					"actual": {
						"time_scale": Engine.time_scale,
						"physics_ticks_per_second": Engine.physics_ticks_per_second,
						"max_fps": Engine.max_fps,
						"max_physics_steps_per_frame": Engine.max_physics_steps_per_frame,
					},
				},
			))
		var payload_or_error: Dictionary = await _finalize_result_payload(true)
		if not bool(payload_or_error.get("write_ok", true)):
			had_artifact_write_failure = true
			var write_failure := _failure(
				"result_write_failed",
				"qa_runner could not write per-test result.json during batch execution",
				{
					"target": str(_check.get("id", "")),
					"expected": "writable result artifact",
					"actual": payload_or_error.get("result_path", ""),
					"artifacts": _artifacts.duplicate(true),
				},
			)
			_failures.append(write_failure)
			aggregate_failures.append(write_failure)
		var payload: Dictionary = payload_or_error.get("payload", {})
		if payload.is_empty():
			payload = _result_payload()
		var payload_failures = payload.get("failures", [])
		var payload_failure_count: int = payload_failures.size() if payload_failures is Array else 0
		_write_check_progress_event(
			"finish",
			entry_check,
			_artifacts,
			batch_artifacts,
			progress_started_at,
			Time.get_ticks_msec() - progress_started_msec,
			bool(payload.get("ok", false)) and payload_failure_count == 0,
			payload_failure_count
		)
		for failure in payload.get("failures", []):
			if failure is Dictionary:
				aggregate_failures.append((failure as Dictionary).duplicate(true))
		summary_entries.append(_batch_summary_entry(payload))
		_clear_bridge_runtime_errors()
		_reset_check_runtime_state()

	_artifacts = batch_artifacts
	_started_at_msec = batch_started_at
	_check = {
		"execution_mode": "batch",
		"tests": tests.duplicate(true),
	}
	_failures = aggregate_failures
	var summary_error := _write_batch_summary(summary_entries)
	if summary_error != OK:
		_failures.append(_failure(
			"batch_summary_write_failed",
			"qa_runner could not write batch summary artifact",
			{
				"target": "batch.summary",
				"expected": "writable batch summary artifact",
				"actual": str(_artifacts.get("summary", "")),
				"artifacts": _artifacts.duplicate(true),
			},
		))
		return await _finalize_result()

	print(JSON.stringify({
		"ok": _failures.is_empty() and not had_artifact_write_failure,
		"execution_mode": "batch",
		"test_count": summary_entries.size(),
		"failures": _failures.duplicate(true),
		"artifacts": _artifacts.duplicate(true),
	}))
	return 0 if _failures.is_empty() and not had_artifact_write_failure else 1


func _run_manifest_check() -> void:
	_check_data = {}
	var test_id := str(_check.get("id", ""))
	var kind := str(_check.get("kind", ""))
	var target := str(_check.get("target", ""))
	if test_id == "" or kind == "" or target == "":
		_failures.append(_failure(
			"check_request_invalid",
			"manifest-backed check requests require id, kind, and target",
			{
				"target": test_id if test_id != "" else "check",
				"expected": {
					"id": "non-empty string",
					"kind": "scene or script",
					"target": "non-empty string",
				},
				"actual": _check.duplicate(true),
			},
		))
		return

	if kind == "scene":
		await _run_manifest_scene_check(test_id, target)
		return
	if kind == "script":
		await _run_manifest_script_check(test_id, target)
		return
	if kind == "script_compile":
		await _run_script_compile_check(test_id, target)
		return

	_failures.append(_failure(
		"check_request_invalid",
		"manifest-backed check kind is unsupported",
		{
			"target": test_id,
			"expected": ["scene", "script", "script_compile"],
			"actual": kind,
		},
	))


func _run_manifest_scene_check(test_id: String, target: String) -> void:
	var scene_load_started := _profile_now_usec()
	var scene_load_failure_count := _failures.size()
	if not await _load_scene_instance(target):
		_record_profile_timing("scene_load", scene_load_started, scene_load_failure_count, {"target": test_id})
		return
	_record_profile_timing("scene_load", scene_load_started, scene_load_failure_count, {"target": test_id})

	var initial_frames_started := _profile_now_usec()
	var initial_frame_failure_count := _failures.size()
	await _tree.process_frame
	_sample_profile_frame()
	RenderingServer.force_draw(false)
	await _tree.process_frame
	_sample_profile_frame()
	_record_profile_timing("initial_rendered_frames", initial_frames_started, initial_frame_failure_count, {
		"target": test_id,
		"frames": 2,
	})

	var startup_frames := max(int(_check.get("startup_frames", 0)), 0)
	var startup_started := _profile_now_usec()
	var startup_failure_count := _failures.size()
	for _frame_index in range(startup_frames):
		await _tree.process_frame
		_sample_profile_frame()
	_record_profile_timing("startup_frames", startup_started, startup_failure_count, {
		"target": test_id,
		"frames": startup_frames,
	})

	var runtime_scan_started := _profile_now_usec()
	var runtime_scan_failure_count := _failures.size()
	if _failures.is_empty() and not _runtime_errors.is_empty():
		_failures.append(_failure(
			"runtime_errors_present",
			"runtime errors were recorded during startup",
			{
				"target": test_id,
				"expected": [],
				"actual": _runtime_errors.duplicate(true),
			},
		))
	_record_profile_timing("runtime_error_scan", runtime_scan_started, runtime_scan_failure_count, {"target": test_id})


func _run_manifest_script_check(test_id: String, target: String) -> void:
	var script_load_started := _profile_now_usec()
	var script_load_failure_count := _failures.size()
	var script_resource := load(target)
	if script_resource == null or not script_resource is GDScript:
		_failures.append(_failure(
			"script_test_load_failed",
			"script test could not be loaded",
			{
				"target": test_id,
				"expected": "loadable GDScript resource",
				"actual": target,
			},
		))
		_record_profile_timing("script_load", script_load_started, script_load_failure_count, {"target": test_id})
		return
	_record_profile_timing("script_load", script_load_started, script_load_failure_count, {"target": test_id})

	var instantiate_started := _profile_now_usec()
	var instantiate_failure_count := _failures.size()
	var script_instance: Variant = (script_resource as GDScript).new()
	script_resource = null
	if script_instance == null:
		_failures.append(_failure(
			"script_test_instantiate_failed",
			"script test could not be instantiated",
			{
				"target": test_id,
				"expected": "instantiated script object",
				"actual": "null",
			},
		))
		_record_profile_timing("script_instantiate", instantiate_started, instantiate_failure_count, {"target": test_id})
		return
	_record_profile_timing("script_instantiate", instantiate_started, instantiate_failure_count, {"target": test_id})
	if not script_instance.has_method("run_qa_test"):
		_failures.append(_failure(
			"script_test_method_missing",
			"script test must define run_qa_test()",
			{
				"target": test_id,
				"expected": "script instance with run_qa_test()",
				"actual": target,
			},
		))
		if script_instance is Node:
			var missing_teardown_started := _profile_now_usec()
			var missing_teardown_failure_count := _failures.size()
			(script_instance as Node).free()
			_record_profile_timing("script_teardown", missing_teardown_started, missing_teardown_failure_count, {"target": test_id})
		script_instance = null
		return

	await _tree.process_frame
	_sample_profile_frame()
	var run_started := _profile_now_usec()
	var run_failure_count := _failures.size()
	var raw_result: Variant = script_instance.call("run_qa_test")
	_record_profile_timing("run_qa_test", run_started, run_failure_count, {"target": test_id})
	await _tree.process_frame
	_sample_profile_frame()
	if script_instance is Node:
		var teardown_started := _profile_now_usec()
		var teardown_failure_count := _failures.size()
		(script_instance as Node).free()
		_record_profile_timing("script_teardown", teardown_started, teardown_failure_count, {"target": test_id})
	script_instance = null
	if not raw_result is Dictionary:
		var validation_started := _profile_now_usec()
		var validation_failure_count := _failures.size()
		_failures.append(_failure(
			"script_test_result_invalid",
			"script test must return a dictionary",
			{
				"target": test_id,
				"expected": "dictionary with ok and failures",
				"actual": raw_result,
			},
		))
		_record_profile_timing("result_validation", validation_started, validation_failure_count, {"target": test_id})
		return

	var validation_started := _profile_now_usec()
	var validation_failure_count := _failures.size()
	var result_dict: Dictionary = raw_result
	var ok_value = result_dict.get("ok", null)
	if not ok_value is bool:
		_failures.append(_failure(
			"script_test_result_invalid",
			"script test result field ok must be a boolean",
			{
				"target": test_id,
				"expected": "boolean",
				"actual": ok_value,
			},
		))
		_record_profile_timing("result_validation", validation_started, validation_failure_count, {"target": test_id})
		return
	var failures_value = result_dict.get("failures", null)
	if not failures_value is Array:
		_failures.append(_failure(
			"script_test_result_invalid",
			"script test result field failures must be an array",
			{
				"target": test_id,
				"expected": "array of dictionaries",
				"actual": failures_value,
			},
		))
		_record_profile_timing("result_validation", validation_started, validation_failure_count, {"target": test_id})
		return

	var data_copy_started := _profile_now_usec()
	var data_copy_failure_count := _failures.size()
	for failure_entry in failures_value:
		if not failure_entry is Dictionary:
			_failures.append(_failure(
				"script_test_result_invalid",
				"script test failure entries must be dictionaries",
				{
					"target": test_id,
					"expected": "dictionary failure entries",
					"actual": failure_entry,
				},
			))
			_record_profile_timing("result_validation", validation_started, validation_failure_count, {"target": test_id})
			return
		_failures.append(_normalize_script_test_failure(test_id, failure_entry))
	_record_profile_timing("result_validation", validation_started, validation_failure_count, {"target": test_id})
	_record_profile_timing("data_copy", data_copy_started, data_copy_failure_count, {
		"target": test_id,
		"failure_count": failures_value.size(),
	})

	if result_dict.has("data"):
		var data_value = result_dict.get("data")
		if data_value is Dictionary:
			_check_data = (data_value as Dictionary).duplicate(true)
		else:
			_failures.append(_failure(
				"script_test_result_invalid",
				"script test result field data must be a dictionary when present",
				{
					"target": test_id,
					"expected": "dictionary",
					"actual": data_value,
				},
			))
			return

	if not bool(ok_value) and failures_value.is_empty():
		_failures.append(_failure(
			"script_test_failed",
			"script test reported failure without failure details",
			{
				"target": test_id,
				"expected": "at least one failure entry when ok is false",
				"actual": [],
			},
		))


func _run_script_compile_check(test_id: String, target: String) -> void:
	var script_load_started := _profile_now_usec()
	var script_load_failure_count := _failures.size()
	var script_resource := load(target)
	if script_resource == null or not script_resource is GDScript:
		_failures.append(_failure(
			"script_load_failed",
			"script could not be loaded as GDScript",
			{
				"target": target,
				"expected": "loadable GDScript resource",
				"actual": target,
			},
		))
		_record_profile_timing("script_load", script_load_started, script_load_failure_count, {"target": test_id})
		return
	_check_data = {
		"status": "passed",
		"validated_resource": target,
	}
	script_resource = null
	_record_profile_timing("script_load", script_load_started, script_load_failure_count, {"target": test_id})


func _normalize_script_test_failure(test_id: String, failure_entry: Dictionary) -> Dictionary:
	var normalized := failure_entry.duplicate(true)
	if not normalized.has("type"):
		normalized["type"] = "script_test_failed"
	if not normalized.has("message"):
		normalized["message"] = "script test reported failures"
	if not normalized.has("target"):
		normalized["target"] = test_id
	if not normalized.has("severity"):
		normalized["severity"] = "error"
	if not normalized.has("artifacts"):
		normalized["artifacts"] = {"run_root": _artifact_run_root()}
	return normalized


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
	if not params is Dictionary:
		return {
			"ok": false,
			"type": "wait_until_timeout",
			"message": "wait_until step requires an object",
			"data": {"target": "wait_until", "expected": "object", "actual": params},
		}
	var config: Dictionary = params
	var timeout_frames := max(int(config.get("timeout_frames", 0)), 0)
	var poll_every_frames := max(int(config.get("poll_every_frames", 10)), 1)
	# Reuse the state_matches single-check machinery: forward only the
	# state-reading keys so the predicate matches `state_matches` semantics.
	var state_params := {}
	for key in [
		"source",
		"provider",
		"provider_id",
		"method",
		"target",
		"path",
		"operator",
		"op",
		"expected",
		"epsilon",
		"tolerance",
		"where",
		"value_path",
	]:
		if config.has(key):
			state_params[key] = config.get(key)
	var elapsed_frames := 0
	var last_error := {}
	while true:
		if _frame_budget_exceeded():
			# Stop consuming frames; the step/assert boundary check owns the
			# authoritative scenario_frame_budget_exceeded failure.
			return {"ok": true, "frames": elapsed_frames, "stop_reason": "frame_budget"}
		var result: Dictionary = await _runtime.execute("assert.state_matches", state_params.duplicate(true))
		if bool(result.get("ok", false)):
			return {"ok": true, "frames": elapsed_frames}
		var error = result.get("error", {})
		if error is Dictionary:
			last_error = error
			var error_type := str((error as Dictionary).get("type", ""))
			# Only a genuine state mismatch means "keep polling". Any other
			# error (unresolved target/source, invalid method, non-JSON result)
			# is a hard failure and aborts immediately.
			if error_type != "" and error_type != "state_matches_mismatch":
				return {
					"ok": false,
					"type": error_type,
					"message": str((error as Dictionary).get("message", "wait_until state source could not be read")),
					"data": (error as Dictionary).get("data", {}),
				}
		if elapsed_frames >= timeout_frames:
			break
		var step_frames := int(min(poll_every_frames, timeout_frames - elapsed_frames))
		for _i in range(step_frames):
			await _tree.process_frame
			_sample_profile_frame()
		elapsed_frames += step_frames
	var data := {
		"target": str(config.get("target", config.get("provider", ""))),
		"path": str(config.get("path", "")),
		"operator": str(config.get("operator", "")),
		"expected": config.get("expected", null),
		"timeout_frames": timeout_frames,
		"poll_every_frames": poll_every_frames,
		"frames_waited": elapsed_frames,
	}
	if last_error is Dictionary and (last_error as Dictionary).has("data"):
		data["last_observed"] = (last_error as Dictionary).get("data", {})
	return {
		"ok": false,
		"type": "wait_until_timeout",
		"message": "wait_until predicate did not hold within timeout_frames",
		"data": data,
	}


# Closed-loop policy-driven input: every tick (each tick_every_frames frames,
# up to max_frames) call the configured policy provider through the runtime,
# emit the joypad inputs it returns for the allowed devices, and check the
# optional `until` predicate at the same cadence. The runner only relays
# policy-computed input through the real input.joy_* event paths; it never
# mutates node state on the policy's behalf. Exhausting max_frames is a
# normal stop (stop_reason: "max_frames"), not a failure — assert the outcome
# with assertions or a following wait_until when the scenario requires it.
func _run_drive_policy(params: Variant, step_index: int) -> Dictionary:
	if not params is Dictionary:
		return {
			"ok": false,
			"type": "drive_policy_failed",
			"message": "drive_policy step requires an object",
			"data": {"target": "drive_policy", "expected": "object", "actual": params},
		}
	var config: Dictionary = params
	var policy := str(config.get("policy", "")).strip_edges()
	var devices = config.get("devices", [])
	var max_frames := int(config.get("max_frames", 0))
	var tick_every_frames := max(int(config.get("tick_every_frames", 6)), 1)
	if policy == "" or not devices is Array or (devices as Array).is_empty() or max_frames <= 0:
		return {
			"ok": false,
			"type": "drive_policy_failed",
			"message": "drive_policy step requires a policy, non-empty devices, and positive max_frames",
			"data": {
				"target": "drive_policy",
				"expected": {"policy": "non-empty string", "devices": "non-empty device list", "max_frames": "positive integer"},
				"actual": config.duplicate(true),
			},
		}
	# Parsed JSON numbers arrive as floats; normalize the device list to ints
	# once for the runtime commands and recorded evidence.
	var device_list: Array = []
	for device_entry in (devices as Array):
		device_list.append(int(device_entry))
	# Reuse the wait_until/state_matches key forwarding so the `until`
	# predicate matches wait_until semantics exactly.
	var until = config.get("until", null)
	var until_params := {}
	if until is Dictionary:
		for key in [
			"source",
			"provider",
			"provider_id",
			"method",
			"target",
			"path",
			"operator",
			"op",
			"expected",
			"epsilon",
			"tolerance",
			"where",
			"value_path",
		]:
			if (until as Dictionary).has(key):
				until_params[key] = (until as Dictionary).get(key)
	var ticks := 0
	var elapsed_frames := 0
	var inputs_applied := 0
	var stop_reason := ""
	var failure := {}
	while true:
		if _frame_budget_exceeded():
			# Stop driving; the step/assert boundary check owns the
			# authoritative scenario_frame_budget_exceeded failure.
			stop_reason = "frame_budget"
			break
		if not until_params.is_empty():
			var until_result: Dictionary = await _runtime.execute("assert.state_matches", until_params.duplicate(true))
			if bool(until_result.get("ok", false)):
				stop_reason = "until_satisfied"
				break
			var until_error = until_result.get("error", {})
			if until_error is Dictionary:
				var error_type := str((until_error as Dictionary).get("type", ""))
				# Match wait_until: only a genuine state mismatch keeps
				# driving. Any other error (unresolved target/source, invalid
				# method, non-JSON result) aborts the step immediately.
				if error_type != "" and error_type != "state_matches_mismatch":
					stop_reason = "until_error"
					failure = {
						"type": error_type,
						"message": str((until_error as Dictionary).get("message", "drive_policy until state source could not be read")),
						"data": (until_error as Dictionary).get("data", {}),
					}
					break
		if elapsed_frames >= max_frames:
			stop_reason = "max_frames"
			break
		var tick_result: Dictionary = await _runtime.execute("policy.drive_tick", {
			"policy": policy,
			"devices": device_list.duplicate(),
			"params": config.get("params", {}),
			"tick": ticks,
			"frame": elapsed_frames,
		})
		if not bool(tick_result.get("ok", false)):
			stop_reason = "policy_error"
			var tick_error = tick_result.get("error", {})
			if tick_error is Dictionary:
				failure = {
					"type": str((tick_error as Dictionary).get("type", "drive_policy_failed")),
					"message": str((tick_error as Dictionary).get("message", "drive_policy policy call failed")),
					"data": (tick_error as Dictionary).get("data", {}),
				}
			else:
				failure = {
					"type": "drive_policy_failed",
					"message": "drive_policy policy call failed",
					"data": {"policy": policy, "tick": ticks},
				}
			break
		ticks += 1
		var tick_data = tick_result.get("data", {})
		if tick_data is Dictionary:
			inputs_applied += int((tick_data as Dictionary).get("inputs_applied", 0))
		var step_frames := int(min(tick_every_frames, max_frames - elapsed_frames))
		for _i in range(step_frames):
			await _tree.process_frame
			_sample_profile_frame()
		elapsed_frames += step_frames
	# Always zero the driven devices' injected axes and release their pressed
	# buttons, even when the step is aborting on a failure.
	var release_result: Dictionary = await _runtime.execute("policy.release_inputs", {"devices": device_list.duplicate()})
	if not bool(release_result.get("ok", false)) and failure.is_empty():
		var release_error = release_result.get("error", {})
		failure = {
			"type": str((release_error as Dictionary).get("type", "drive_policy_failed")) if release_error is Dictionary else "drive_policy_failed",
			"message": str((release_error as Dictionary).get("message", "drive_policy could not release driven inputs")) if release_error is Dictionary else "drive_policy could not release driven inputs",
			"data": (release_error as Dictionary).get("data", {}) if release_error is Dictionary else {"policy": policy},
		}
	_policy_drives[str(step_index)] = {
		"step_index": step_index,
		"policy": policy,
		"devices": device_list.duplicate(),
		"ticks": ticks,
		"frames": elapsed_frames,
		"inputs_applied": inputs_applied,
		"stop_reason": stop_reason,
	}
	if not failure.is_empty():
		return {
			"ok": false,
			"type": str(failure.get("type", "drive_policy_failed")),
			"message": str(failure.get("message", "drive_policy step failed")),
			"data": failure.get("data", {}),
			"ticks": ticks,
			"frames": elapsed_frames,
			"stop_reason": stop_reason,
		}
	return {"ok": true, "ticks": ticks, "frames": elapsed_frames, "stop_reason": stop_reason}


func _run_assertion(assertion: Variant, assertion_index := -1, runtime_path := "") -> void:
	var started_usec := _progress_now_usec()
	var failure_count := _failures.size()
	_active_assertion_source_id = _assertion_proof.begin(runtime_path) if _assertion_proof != null else ""
	var timing_extra := {
		"index": assertion_index,
		"kind": "assert",
	}
	var heartbeat_context := _progress_context_for_operation(assertion, "assertion")
	var heartbeat_command := ""
	if assertion is Dictionary:
		var heartbeat_runtime_command := _runtime_command_for_assertion(assertion)
		if not heartbeat_runtime_command.is_empty():
			heartbeat_command = str(heartbeat_runtime_command.get("command", ""))
			heartbeat_context = heartbeat_runtime_command.get("progress_context", heartbeat_context)
	var heartbeat_started_at := _progress_started_at()
	_write_scenario_heartbeat("start", "assertion", assertion_index, "assert", heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)
	if not assertion is Dictionary or _assertion_operation_keys(assertion).size() != 1:
		_failures.append(_failure(
			"unsupported_assertion",
			"assertion must be an object with exactly one operation",
			{
				"target": "assertion",
				"expected": "object with exactly one supported operation",
				"actual": assertion,
			},
		))
		_record_profile_timing("assertion", started_usec, failure_count, timing_extra)
		_assertion_proof.finish(runtime_path, _failures.size() == failure_count)
		_write_scenario_heartbeat("finish", "assertion", assertion_index, "assert", heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)
		return

	var runtime_command := _runtime_command_for_assertion(assertion)
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
				"index": assertion_index,
				"kind": "runtime_request",
				"command": str(runtime_command.get("command", "")),
			},
		)
		if not bool(runtime_result.get("ok", false)):
			_failures.append(_runtime_error_to_failure(runtime_result.get("error", {})))
		_record_profile_timing("assertion", started_usec, failure_count, timing_extra)
		_assertion_proof.finish(runtime_path, _failures.size() == failure_count)
		_write_scenario_heartbeat("finish", "assertion", assertion_index, "assert", str(runtime_command.get("command", "")), heartbeat_context, heartbeat_started_at, started_usec, failure_count)
		return

	_failures.append(_failure(
		"unsupported_assertion",
		"M03 runner does not support this assertion yet",
		{
			"target": _operation_name(assertion, "assertion"),
			"expected": EXPANDED_ASSERTION_TYPES,
			"actual": assertion,
		},
	))
	_record_profile_timing("assertion", started_usec, failure_count, timing_extra)
	_assertion_proof.finish(runtime_path, _failures.size() == failure_count)
	_write_scenario_heartbeat("finish", "assertion", assertion_index, "assert", heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)


func _runtime_command_for_step(step: Dictionary) -> Dictionary:
	if step.has("tap_action"):
		return {
			"command": "input.tap_action",
			"params": {
				"action": str(step.get("tap_action", "")),
			},
		}
	if step.has("hold_action"):
		var hold_action: Dictionary = step.get("hold_action", {})
		return {
			"command": "input.hold_action",
			"params": {
				"action": str(hold_action.get("action", "")),
				"frames": hold_action.get("frames", 0),
			},
		}
	if step.has("release_action"):
		return {
			"command": "input.release_action",
			"params": {
				"action": str(step.get("release_action", "")),
			},
		}
	if step.has("input_joy_button"):
		var joy_button = step.get("input_joy_button", "")
		if joy_button is Dictionary:
			return {
				"command": "input.joy_button",
				"params": {
					"button": str(joy_button.get("button", "")),
					"device": joy_button.get("device", 0),
					"frames": joy_button.get("frames", 1),
				},
			}
		return {
			"command": "input.joy_button",
			"params": {
				"button": str(joy_button),
				"device": 0,
				"frames": 1,
			},
		}
	if step.has("input_joy_motion"):
		var joy_motion: Dictionary = step.get("input_joy_motion", {})
		return {
			"command": "input.joy_motion",
			"params": {
				"axis": str(joy_motion.get("axis", "")),
				"value": joy_motion.get("value", null),
				"device": joy_motion.get("device", 0),
				"frames": joy_motion.get("frames", 1),
			},
		}
	if step.has("input_joy_state"):
		var joy_state: Dictionary = step.get("input_joy_state", {})
		return {
			"command": "input.joy_state",
			"params": {
				"inputs": joy_state.get("inputs", []),
				"frames": joy_state.get("frames", 1),
			},
		}
	if step.has("joy_connect"):
		var joy_connect: Dictionary = step.get("joy_connect", {}) if step.get("joy_connect", {}) is Dictionary else {}
		var connect_params := {"devices": joy_connect.get("devices", [])}
		if joy_connect.has("name"):
			connect_params["name"] = joy_connect.get("name")
		return {
			"command": "setup.joy_connect",
			"params": connect_params,
		}
	if step.has("joy_disconnect"):
		var joy_disconnect: Dictionary = step.get("joy_disconnect", {}) if step.get("joy_disconnect", {}) is Dictionary else {}
		return {
			"command": "setup.joy_disconnect",
			"params": {"devices": joy_disconnect.get("devices", [])},
		}
	if step.has("click"):
		return {
			"command": "input.click_target",
			"params": {
				"target": str(step.get("click", "")),
			},
			"progress_context": {"target": str(step.get("click", ""))},
		}
	if step.has("node_inspect"):
		return {
			"command": "node.inspect",
			"params": {
				"target": str(step.get("node_inspect", "")),
			},
			"progress_context": {"target": str(step.get("node_inspect", ""))},
		}
	if step.has("record_state"):
		var record_state: Dictionary = step.get("record_state", {}) if step.get("record_state", {}) is Dictionary else {}
		return {
			"command": "state.record",
			"params": record_state.duplicate(true),
			"progress_context": {"name": str(record_state.get("name", ""))},
		}
	if step.has("physics_overlay"):
		var overlay_params := _physics_overlay_step_params(step.get("physics_overlay", {}))
		return {
			"command": "physics.overlay.collect",
			"params": overlay_params,
			"progress_context": _safe_progress_context(overlay_params),
		}
	if step.has("teleport_node"):
		var teleport_node: Dictionary = step.get("teleport_node", {})
		return {
			"command": "setup.teleport_node",
			"params": teleport_node,
			"progress_context": _safe_progress_context(teleport_node),
		}
	if step.has("fixture_teleport"):
		var fixture_teleport: Dictionary = step.get("fixture_teleport", {})
		return {
			"command": "fixture.teleport_to_target",
			"params": fixture_teleport,
			"progress_context": _safe_progress_context(fixture_teleport),
		}
	if step.has("spawn_scene"):
		var spawn_scene: Dictionary = step.get("spawn_scene", {})
		return {
			"command": "setup.spawn_scene",
			"params": spawn_scene,
			"progress_context": _safe_progress_context(spawn_scene),
		}
	if step.has("set_property"):
		var set_property: Dictionary = step.get("set_property", {})
		return {
			"command": "setup.set_property",
			"params": set_property,
			"progress_context": _safe_progress_context(set_property),
		}
	if step.has("call_method"):
		var call_method: Dictionary = step.get("call_method", {})
		return {
			"command": "setup.call_method",
			"params": call_method,
			"progress_context": _safe_progress_context(call_method),
		}
	if step.has("fixture_call"):
		var fixture_call: Dictionary = step.get("fixture_call", {})
		return {
			"command": "fixture.call",
			"params": fixture_call,
			"progress_context": _safe_progress_context(fixture_call),
		}
	return {}




func _temporal_params_with_artifacts(params: Dictionary) -> Dictionary:
	var merged := params.duplicate(true)
	if not _scenario_temporal_sampling.is_empty():
		merged["temporal_sampling"] = _scenario_temporal_sampling.duplicate(true)
	if params.get("temporal_sampling", null) is Dictionary:
		merged["temporal_sampling"] = (params.get("temporal_sampling", {}) as Dictionary).duplicate(true)
	var run_root := _artifact_run_root()
	if run_root != "":
		merged["temporal_dir"] = "%s/temporal" % run_root
	return merged

func _runtime_command_for_assertion(assertion: Dictionary) -> Dictionary:
	var visual_dir := ""
	var run_root := _artifact_run_root()
	if run_root != "":
		visual_dir = "%s/visual" % run_root
	if assertion.has("exists"):
		return {"command": "assert.exists", "params": {"target": str(assertion.get("exists", ""))}, "progress_context": {"target": str(assertion.get("exists", ""))}}
	if assertion.has("not_exists"):
		return {"command": "assert.not_exists", "params": {"target": str(assertion.get("not_exists", ""))}, "progress_context": {"target": str(assertion.get("not_exists", ""))}}
	if assertion.has("visible"):
		return {"command": "assert.visible", "params": {"target": str(assertion.get("visible", ""))}, "progress_context": {"target": str(assertion.get("visible", ""))}}
	if assertion.has("hidden"):
		return {"command": "assert.hidden", "params": {"target": str(assertion.get("hidden", ""))}, "progress_context": {"target": str(assertion.get("hidden", ""))}}
	if assertion.has("enabled"):
		return {"command": "assert.enabled", "params": {"target": str(assertion.get("enabled", ""))}, "progress_context": {"target": str(assertion.get("enabled", ""))}}
	if assertion.has("disabled"):
		return {"command": "assert.disabled", "params": {"target": str(assertion.get("disabled", ""))}, "progress_context": {"target": str(assertion.get("disabled", ""))}}
	if assertion.has("focused"):
		return {"command": "assert.focused", "params": {"target": str(assertion.get("focused", ""))}, "progress_context": {"target": str(assertion.get("focused", ""))}}
	if assertion.has("text_equals"):
		var text_equals: Dictionary = assertion.get("text_equals", {})
		return {
			"command": "assert.text_equals",
			"params": {
				"target": str(text_equals.get("target", "")),
				"expected": str(text_equals.get("expected", "")),
			},
			"progress_context": {"target": str(text_equals.get("target", ""))},
		}
	if assertion.has("text_contains"):
		var text_contains: Dictionary = assertion.get("text_contains", {})
		return {
			"command": "assert.text_contains",
			"params": {
				"target": str(text_contains.get("target", "")),
				"expected": str(text_contains.get("expected", "")),
			},
			"progress_context": {"target": str(text_contains.get("target", ""))},
		}
	if assertion.has("inside_viewport"):
		return {
			"command": "assert.inside_viewport",
			"params": {"target": str(assertion.get("inside_viewport", ""))},
			"progress_context": {"target": str(assertion.get("inside_viewport", ""))},
		}
	if assertion.has("no_overlap"):
		var no_overlap: Dictionary = assertion.get("no_overlap", {})
		return {
			"command": "assert.no_overlap",
			"params": {
				"a": str(no_overlap.get("a", "")),
				"b": str(no_overlap.get("b", "")),
			},
			"progress_context": _safe_progress_context(no_overlap),
		}
	if assertion.has("distance"):
		var distance: Dictionary = assertion.get("distance", {})
		return {
			"command": "assert.distance",
			"params": distance,
		}
	if assertion.has("distance_between"):
		var distance_between: Dictionary = assertion.get("distance_between", {})
		var distance_between_params := distance_between.duplicate(true)
		if distance_between_params.has("min_distance") and not distance_between_params.has("min"):
			distance_between_params["min"] = distance_between_params.get("min_distance")
		if distance_between_params.has("max_distance") and not distance_between_params.has("max"):
			distance_between_params["max"] = distance_between_params.get("max_distance")
		return {
			"command": "assert.distance",
			"params": distance_between_params,
		}
	if assertion.has("distance_changed"):
		return {
			"command": "assert.temporal.distance_changed",
			"params": _temporal_params_with_artifacts(assertion.get("distance_changed", {})),
		}
	if assertion.has("observe_until"):
		return {
			"command": "assert.observe_until",
			"params": _temporal_params_with_artifacts(assertion.get("observe_until", {})),
		}
	if assertion.has("state_matches"):
		var state_matches: Dictionary = assertion.get("state_matches", {})
		return {"command": "assert.state_matches", "params": state_matches.duplicate(true)}
	if assertion.has("surface_probe"):
		var surface_probe: Dictionary = assertion.get("surface_probe", {})
		return {"command": "assert.surface_probe", "params": surface_probe}
	if assertion.has("surface_matches"):
		var surface_matches: Dictionary = assertion.get("surface_matches", {})
		return {"command": "assert.surface_matches", "params": _temporal_params_with_artifacts(surface_matches)}
	if assertion.has("visible_pixels"):
		var visible_pixels: Dictionary = assertion.get("visible_pixels", {})
		var params := visible_pixels.duplicate(true)
		params["target"] = str(visible_pixels.get("target", ""))
		params["visual_dir"] = visual_dir
		return {"command": "assert.visible_pixels", "params": params}
	if assertion.has("not_occluded"):
		var not_occluded: Dictionary = assertion.get("not_occluded", {})
		var no_params := not_occluded.duplicate(true)
		no_params["target"] = str(not_occluded.get("target", ""))
		no_params["visual_dir"] = visual_dir
		return {"command": "assert.not_occluded", "params": no_params}
	if assertion.has("renders_above"):
		var renders_above: Dictionary = assertion.get("renders_above", {})
		var ra_params := renders_above.duplicate(true)
		if ra_params.has("a") and not ra_params.has("target"):
			ra_params["target"] = ra_params.get("a")
		if ra_params.has("b") and not ra_params.has("other"):
			ra_params["other"] = ra_params.get("b")
		if str(ra_params.get("snapshot", "")) == "" and _last_snapshot_name != "":
			ra_params["snapshot"] = _last_snapshot_name
		ra_params["visual_dir"] = visual_dir
		return {"command": "assert.renders_above", "params": ra_params}
	if assertion.has("renders_below"):
		var renders_below: Dictionary = assertion.get("renders_below", {})
		var rb_params := renders_below.duplicate(true)
		if rb_params.has("a") and not rb_params.has("target"):
			rb_params["target"] = rb_params.get("a")
		if rb_params.has("b") and not rb_params.has("other"):
			rb_params["other"] = rb_params.get("b")
		if str(rb_params.get("snapshot", "")) == "" and _last_snapshot_name != "":
			rb_params["snapshot"] = _last_snapshot_name
		rb_params["visual_dir"] = visual_dir
		return {"command": "assert.renders_below", "params": rb_params}
	if assertion.has("no_errors"):
		return {"command": "assert.no_errors", "params": {}}
	if assertion.has("max_delta_position"):
		var max_delta_position: Dictionary = assertion.get("max_delta_position", {})
		var max_delta_params := max_delta_position.duplicate(true)
		if max_delta_params.has("max_delta") and not max_delta_params.has("max"):
			max_delta_params["max"] = max_delta_params.get("max_delta")
		if not max_delta_params.has("window_frames") and not max_delta_params.has("window_seconds"):
			max_delta_params["window_frames"] = 1
		return {"command": "assert.temporal.max_delta_position", "params": _temporal_params_with_artifacts(max_delta_params)}
	if assertion.has("follows"):
		return {"command": "assert.temporal.follows", "params": _temporal_params_with_artifacts(assertion.get("follows", {}))}
	if assertion.has("never"):
		return {"command": "assert.temporal.never", "params": _temporal_params_with_artifacts(assertion.get("never", {}))}
	if assertion.has("within_limits"):
		return {"command": "assert.temporal.within_limits", "params": _temporal_params_with_artifacts(assertion.get("within_limits", {}))}
	if assertion.has("trend"):
		return {"command": "assert.temporal.trend", "params": _temporal_params_with_artifacts(assertion.get("trend", {}))}
	if assertion.has("frame_time"):
		return {"command": "assert.temporal.frame_time", "params": _temporal_params_with_artifacts(assertion.get("frame_time", {}))}
	return {}


func _operation_name(entry: Dictionary, fallback: String) -> String:
	var keys := _assertion_operation_keys(entry)
	if keys.size() != 1:
		return fallback
	if keys.is_empty():
		return fallback
	return str(keys[0])


func _assertion_operation_keys(entry: Dictionary) -> Array:
	var operation_keys: Array = []
	for key in entry.keys():
		if str(key) == "temporal":
			continue
		operation_keys.append(key)
	return operation_keys


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
	_profile_enabled = false
	_profile_max_frame_samples = 0
	_profile_last_frame_usec = 0
	_profile_frame_times_ms = []
	_profile_frame_samples = []
	_profile_monitor_samples = []
	_profile_unsupported_monitors = []
	_profile_step_timings = []
	_profile_top_script_methods = 10
	_profile_next_step_index = 0
	_profile_next_assertion_index = 0
	_profile_summary = {}
	var profile_request = request.get("profiling", request.get("profile", {}))
	if not profile_request is Dictionary:
		return
	_profile_enabled = bool(profile_request.get("enabled", false))
	if not _profile_enabled:
		return
	QaProfiler.reset()
	_profile_max_frame_samples = max(int(profile_request.get("max_frame_samples", 120)), 0)
	_profile_top_script_methods = max(int(profile_request.get("top", 10)), 0)
	_profile_last_frame_usec = Time.get_ticks_usec()
	_profile_summary = {
		"started_usec": _profile_last_frame_usec,
		"max_frame_samples": _profile_max_frame_samples,
		"top_script_methods": _profile_top_script_methods,
	}


func _profile_now_usec() -> int:
	return Time.get_ticks_usec() if _profile_enabled else 0


# Record an always-on coarse timing phase (independent of the profile gate).
# started_usec must come from Time.get_ticks_usec(), not _profile_now_usec().
func _mark_timing(key: String, started_usec: int) -> void:
	_timing[key] = float(max(Time.get_ticks_usec() - started_usec, 0)) / 1000.0


func _record_profile_timing(label: String, started_usec: int, failure_count: int, extra: Dictionary) -> void:
	if not _profile_enabled:
		return
	var entry := extra.duplicate(true)
	entry["label"] = label
	entry["duration_ms"] = float(max(Time.get_ticks_usec() - started_usec, 0)) / 1000.0
	entry["failure_count_delta"] = max(_failures.size() - failure_count, 0)
	_profile_step_timings.append(entry)


func _profile_step_kind(step: Variant) -> String:
	if not step is Dictionary:
		return "unsupported"
	if step.has("wait_frames"):
		return "wait_frames"
	if step.has("wait_until"):
		return "wait_until"
	if step.has("drive_policy"):
		return "drive_policy"
	if step.has("snapshot"):
		return "snapshot"
	if step.has("assert"):
		return "assert"
	if step.has("audio_capture"):
		return "audio_capture"
	if step.has("audio_export"):
		return "audio_export"
	var runtime_command := _runtime_command_for_step(step)
	if runtime_command.is_empty():
		return "unsupported"
	var command := str(runtime_command.get("command", ""))
	if command.begins_with("input."):
		return "input"
	if command.begins_with("setup.") or command.begins_with("fixture."):
		return "setup"
	return "runtime_request"


func _record_node_inspection(step_index: int, target: String, result: Dictionary) -> void:
	var data = result.get("data", {})
	if not data is Dictionary:
		return
	var inspection: Dictionary = (data as Dictionary).duplicate(true)
	inspection["step_index"] = step_index
	inspection["target"] = target
	_node_inspections["%d:%s" % [step_index, target]] = inspection


# record_state evidence is keyed by the step's `name` (validated unique per
# scenario doc-side), mirroring how node_inspect keys `_node_inspections`. The
# bounded value plus its source descriptor come straight from the state.record
# runtime response; we only stamp the step index and name so the evidence stays
# addressable in result.json and the sweep pass-through.
func _record_state_record(step_index: int, name: String, result: Dictionary) -> void:
	var data = result.get("data", {})
	if not data is Dictionary:
		return
	var record: Dictionary = (data as Dictionary).duplicate(true)
	record["step_index"] = step_index
	record["name"] = name
	_state_records[name] = record


func _physics_overlay_step_params(raw_params: Variant) -> Dictionary:
	var params: Dictionary = {}
	if raw_params is Dictionary:
		params = (raw_params as Dictionary).duplicate(true)
	else:
		params["target"] = str(raw_params)
	if not params.has("target"):
		params["target"] = ""
	params["target"] = str(params.get("target", "")).strip_edges()
	params["viewport"] = bool(params.get("viewport", false))
	if params.has("radius"):
		params["radius"] = clampf(float(params.get("radius", 256.0)), 1.0, 4096.0)
	if params.has("frames"):
		params["frames"] = clampi(int(params.get("frames", 1)), 1, 240)
	if params.has("include_aabb"):
		params["include_aabb"] = bool(params.get("include_aabb", true))
	if params.has("include_contacts"):
		params["include_contacts"] = bool(params.get("include_contacts", false))
	if params.has("max_shapes"):
		params["max_shapes"] = clampi(int(params.get("max_shapes", 512)), 1, 4096)
	if params.has("state_method") and not params.has("state_methods"):
		params["state_methods"] = params.get("state_method")
	return params


func _record_physics_overlay(step_index: int, step: Dictionary, params: Dictionary, result: Dictionary) -> void:
	if _physics_overlays.size() >= MAX_PHYSICS_OVERLAY_EVIDENCE:
		return
	var data = result.get("data", {})
	if not data is Dictionary:
		return
	var raw_step = step.get("physics_overlay", {})
	var step_params: Dictionary = raw_step if raw_step is Dictionary else {}
	var name := str(step_params.get("name", "")).strip_edges()
	var target := str(params.get("target", data.get("target", ""))).strip_edges()
	var key_target := "viewport" if bool(params.get("viewport", false)) else target
	var key := "%d:%s" % [step_index, name if name != "" else key_target]
	var evidence := {
		"step_index": step_index,
		"name": name,
		"target": target,
		"viewport": bool(params.get("viewport", false)),
		"metadata": (data as Dictionary).duplicate(true),
	}
	var snapshot := str(step_params.get("snapshot", "")).strip_edges()
	if snapshot == "":
		snapshot = _last_snapshot_name
	if snapshot != "":
		evidence["snapshot"] = snapshot
	_physics_overlays[key] = evidence


func _sample_profile_frame() -> void:
	if not _profile_enabled:
		return
	var now_usec := Time.get_ticks_usec()
	var elapsed_ms := float(max(now_usec - _profile_last_frame_usec, 0)) / 1000.0
	_profile_last_frame_usec = now_usec
	if _profile_frame_times_ms.size() < _profile_max_frame_samples:
		_profile_frame_times_ms.append(elapsed_ms)
		_profile_frame_samples.append({
			"index": _profile_frame_samples.size(),
			"timestamp_usec": now_usec,
			"duration_ms": elapsed_ms,
		})
	for monitor_name in PROFILE_MONITORS.keys():
		var monitor_id: int = PROFILE_MONITORS[monitor_name]
		var value = Performance.get_monitor(monitor_id)
		if value == null:
			if not _profile_unsupported_monitors.has(monitor_name):
				_profile_unsupported_monitors.append(monitor_name)
			continue
		if _profile_monitor_samples.size() < _profile_max_frame_samples * PROFILE_MONITORS.size():
			_profile_monitor_samples.append({
				"name": monitor_name,
				"value": float(value),
				"frame_index": max(_profile_frame_samples.size() - 1, 0),
				"timestamp_usec": now_usec,
			})


func _profile_payload() -> Dictionary:
	return {
		"enabled": true,
		"frame_times_ms": _profile_frame_times_ms.duplicate(),
		"frame_samples": _profile_frame_samples.duplicate(true),
		"monitor_samples": _profile_monitor_samples.duplicate(true),
		"unsupported_monitors": _profile_unsupported_monitors.duplicate(),
		"step_timings": _profile_step_timings.duplicate(true),
		"top_script_methods": QaProfiler.snapshot(_profile_top_script_methods, false),
		"summary": {
			"sample_count": _profile_frame_times_ms.size(),
			"monitor_sample_count": _profile_monitor_samples.size(),
			"step_timing_count": _profile_step_timings.size(),
			"max_frame_samples": _profile_max_frame_samples,
			"top_script_method_limit": _profile_top_script_methods,
			"unsupported_monitor_count": _profile_unsupported_monitors.size(),
		},
	}


func _capture_snapshot(snapshot_name: String) -> void:
	var started_usec := _profile_now_usec()
	var failure_count := _failures.size()
	if not _snapshots.has(snapshot_name):
		_snapshots.append(snapshot_name)
	_last_snapshot_name = snapshot_name

	var snapshot_dir := _snapshot_dir(snapshot_name)
	var snapshot_paths := QaSnapshot.snapshot_paths(snapshot_dir)
	_snapshot_artifacts[snapshot_name] = snapshot_paths.duplicate(true)
	_last_snapshot_artifacts = snapshot_paths.duplicate(true)
	var filter_params := {}
	if _scenario.get("snapshots", {}) is Dictionary:
		filter_params = (_scenario.get("snapshots", {}) as Dictionary).duplicate(true)
	var snapshot_scope: Dictionary = _runtime.resolve_snapshot_filter(filter_params)
	if not bool(snapshot_scope.get("ok", false)):
		var scope_error: Dictionary = snapshot_scope.get("error", {})
		_failures.append(_failure(
			str(scope_error.get("type", "invalid_snapshot_filter")),
			str(scope_error.get("message", "snapshot filter could not be resolved")),
			scope_error,
		))
		_record_profile_timing("snapshot_capture", started_usec, failure_count, {
			"kind": "snapshot",
			"snapshot": snapshot_name,
			"artifacts": ["scope"],
		})
		return
	if _asset_trial != null:
		_asset_trial.snapshot_begin(_runtime, snapshot_name)
	var capture_result: Dictionary = await QaSnapshot.capture_snapshot_bundle(
		snapshot_scope.get("root"),
		_tree.get_root(),
		_runtime_errors,
		snapshot_dir,
		snapshot_scope.get("options", {}),
	)
	if _asset_trial != null:
		await _asset_trial.snapshot(_runtime, snapshot_name, snapshot_dir.path_join("asset-trial-visual"))
	if bool(capture_result.get("ok", false)):
		_record_profile_timing("snapshot_capture", started_usec, failure_count, {
			"kind": "snapshot",
			"snapshot": snapshot_name,
			"artifacts": ["screenshot", "tree", "ui", "focus", "errors"],
		})
		return
	for failure in capture_result.get("failures", []):
		if failure is Dictionary:
			var failure_dict: Dictionary = (failure as Dictionary).duplicate(true)
			_record_runtime_error("Unable to capture snapshot artifact %s" % str(failure_dict.get("target", snapshot_name)))
			_failures.append(_failure(
				str(failure_dict.get("type", "snapshot_write_failed")),
				str(failure_dict.get("message", "snapshot artifact failed")),
				{
					"target": failure_dict.get("target"),
					"expected": failure_dict.get("expected"),
					"actual": failure_dict.get("actual"),
				},
			))
	_record_profile_timing("snapshot_capture", started_usec, failure_count, {
		"kind": "snapshot",
		"snapshot": snapshot_name,
		"artifacts": ["screenshot", "tree", "ui", "focus", "errors"],
	})


func _run_audio_capture_step(params: Dictionary) -> void:
	var name := str(params.get("name", ""))
	var safe_name := _safe_audio_artifact_name(name)
	var bus := str(params.get("bus", "Master"))
	var duration_seconds := float(params.get("duration_seconds", 0.0))
	var audio_dir := _audio_artifact_dir()
	var wav_path := _audio_wav_path(safe_name)
	if safe_name == "" or audio_dir == "" or duration_seconds <= 0.0 or not is_finite(duration_seconds):
		_failures.append(_audio_failure(
			"audio_capture_failed",
			"audio_capture requires a non-empty name, controlled audio_dir, and positive duration_seconds",
			"audio_capture",
			{
				"target": bus,
				"expected": "valid name, artifacts.audio_dir, duration_seconds > 0",
				"actual": params.duplicate(true),
			},
		))
		return
	var bus_index := AudioServer.get_bus_index(bus)
	if bus_index < 0:
		_failures.append(_audio_failure(
			"audio_capture_unavailable",
			"audio_capture could not find the requested bus",
			"audio_capture",
			{
				"target": bus,
				"expected": "existing AudioServer bus",
				"actual": "missing",
			},
		))
		return
	var record_effect := AudioEffectRecord.new()
	if record_effect == null or not record_effect.has_method("set_recording_active") or not record_effect.has_method("get_recording"):
		_failures.append(_audio_failure(
			"audio_capture_unavailable",
			"Godot audio recording APIs are unavailable",
			"audio_capture",
			{
				"target": bus,
				"expected": "AudioEffectRecord with recording API",
				"actual": "unavailable",
			},
		))
		return
	var effect_index := AudioServer.get_bus_effect_count(bus_index)
	var added_effect := false
	# Globalize so a user:// audio_dir (Android scenario sessions) resolves to a real
	# OS path; a desktop absolute path passes through globalize_path unchanged.
	var dir_error := DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(audio_dir))
	if dir_error != OK:
		_failures.append(_audio_failure(
			"audio_capture_failed",
			"audio_capture could not create the controlled audio artifact directory",
			"audio_capture",
			{
				"target": bus,
				"expected": audio_dir,
				"actual": dir_error,
			},
		))
		return
	AudioServer.add_bus_effect(bus_index, record_effect, effect_index)
	added_effect = true
	record_effect.call("set_recording_active", true)
	if _asset_trial != null:
		_asset_trial.audio_start(_runtime, name, bus)
	var capture_started_msec := Time.get_ticks_msec()
	while float(Time.get_ticks_msec() - capture_started_msec) / 1000.0 < duration_seconds:
		await _tree.process_frame
		_sample_profile_frame()
		if _asset_trial != null:
			_asset_trial.audio_frame(_runtime)
	record_effect.call("set_recording_active", false)
	if _asset_trial != null:
		_asset_trial.audio_end(_runtime)
	var recording = record_effect.call("get_recording")
	if added_effect:
		AudioServer.remove_bus_effect(bus_index, effect_index)
	record_effect = null
	if recording == null or not recording is AudioStreamWAV:
		_failures.append(_audio_failure(
			"audio_capture_failed",
			"audio_capture did not produce a WAV recording",
			"audio_capture",
			{
				"target": bus,
				"expected": "AudioStreamWAV recording",
				"actual": recording,
			},
		))
		return
	var save_error: int = (recording as AudioStreamWAV).save_to_wav(wav_path)
	if save_error != OK or not FileAccess.file_exists(wav_path):
		_failures.append(_audio_failure(
			"audio_capture_failed",
			"audio_capture could not write the WAV artifact",
			"audio_capture",
			{
				"target": bus,
				"expected": wav_path,
				"actual": save_error,
			},
		))
		return
	_record_audio_artifact(name, safe_name, wav_path, {"source": "audio_capture", "bus": bus, "duration_seconds": duration_seconds})


func _run_audio_export_step(params: Dictionary) -> void:
	var name := str(params.get("name", ""))
	var safe_name := _safe_audio_artifact_name(name)
	var target := str(params.get("target", ""))
	var method_name := str(params.get("method", ""))
	var audio_dir := _audio_artifact_dir()
	var wav_path := _audio_wav_path(safe_name)
	if safe_name == "" or target == "" or audio_dir == "":
		_failures.append(_audio_failure(
			"audio_export_invalid",
			"audio_export requires a non-empty name, target, and controlled audio_dir",
			"audio_export",
			{
				"target": target,
				"expected": "valid name, target, and artifacts.audio_dir",
				"actual": params.duplicate(true),
			},
		))
		return
	if not method_name.begins_with("qa_"):
		_failures.append(_audio_failure(
			"audio_export_rejected",
			"audio_export only allows qa_ methods",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "method name beginning with qa_",
				"actual": method_name,
			},
		))
		return
	var resolution: Dictionary = _runtime.resolve_target(target)
	if not bool(resolution.get("ok", false)):
		_failures.append(_runtime_error_to_failure(resolution.get("error", {})))
		return
	var node := resolution.get("node") as Node
	if node == null or not node.has_method(method_name):
		_failures.append(_audio_failure(
			"audio_export_rejected",
			"audio_export could not find the requested qa_ method",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "method on resolved node",
				"actual": node.get_class() if node != null else "missing",
			},
		))
		return
	# Globalize so a user:// audio_dir (Android scenario sessions) resolves to a real
	# OS path; a desktop absolute path passes through globalize_path unchanged.
	var dir_error := DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(audio_dir))
	if dir_error != OK:
		_failures.append(_audio_failure(
			"audio_export_failed",
			"audio_export could not create the controlled audio artifact directory",
			"audio_export",
			{
				"target": target,
				"expected": audio_dir,
				"actual": dir_error,
			},
		))
		return
	var args_value = params.get("args", [])
	var args: Array = args_value if args_value is Array else []
	var call_args := args.duplicate(true)
	call_args.append({
		"audio_dir": audio_dir,
		"name": safe_name,
		"wav_path": wav_path,
		"max_bytes": AUDIO_EXPORT_MAX_BYTES,
	})
	var result = node.callv(method_name, call_args)
	await _tree.process_frame
	if result is PackedByteArray:
		_accept_audio_export_bytes(name, safe_name, wav_path, result, target, method_name)
		return
	if result is String:
		var decoded := Marshalls.base64_to_raw(str(result))
		_accept_audio_export_bytes(name, safe_name, wav_path, decoded, target, method_name)
		return
	if result is Dictionary:
		_accept_audio_export_dictionary(name, safe_name, result, target, method_name)
		return
	_failures.append(_audio_failure(
		"audio_export_invalid_return",
		"audio_export method returned an unsupported value",
		"audio_export",
		{
			"target": target,
			"method": method_name,
			"expected": "base64 WAV bytes, PackedByteArray, or dictionary describing a WAV under audio_dir",
			"actual": result,
		},
	))


func _accept_audio_export_bytes(name: String, safe_name: String, wav_path: String, bytes: PackedByteArray, target: String, method_name: String) -> void:
	if bytes.size() <= 0 or bytes.size() > AUDIO_EXPORT_MAX_BYTES:
		_failures.append(_audio_failure(
			"audio_export_oversized",
			"audio_export returned an empty or oversized WAV payload",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "1..%d bytes" % AUDIO_EXPORT_MAX_BYTES,
				"actual": bytes.size(),
			},
		))
		return
	if not _looks_like_wav(bytes):
		_failures.append(_audio_failure(
			"audio_export_invalid_return",
			"audio_export returned bytes that are not RIFF/WAVE data",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "RIFF/WAVE bytes",
				"actual": "invalid_header",
			},
		))
		return
	var file := FileAccess.open(wav_path, FileAccess.WRITE)
	if file == null:
		_failures.append(_audio_failure(
			"audio_export_failed",
			"audio_export could not write the WAV artifact",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": wav_path,
				"actual": "open_failed",
			},
		))
		return
	file.store_buffer(bytes)
	_record_audio_artifact(name, safe_name, wav_path, {"source": "audio_export", "target": target, "method": method_name})


func _accept_audio_export_dictionary(name: String, safe_name: String, result: Dictionary, target: String, method_name: String) -> void:
	if not _is_json_compatible(result):
		_failures.append(_audio_failure(
			"audio_export_invalid_metadata",
			"audio_export returned non-JSON-compatible metadata",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "JSON-compatible dictionary",
				"actual": result,
			},
		))
		return
	var wav_path := str(result.get("wav", result.get("path", "")))
	if not _is_path_under_audio_dir(wav_path):
		_failures.append(_audio_failure(
			"audio_export_path_rejected",
			"audio_export returned a WAV path outside the controlled audio directory",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": _audio_artifact_dir(),
				"actual": wav_path,
			},
		))
		return
	if not FileAccess.file_exists(wav_path):
		_failures.append(_audio_failure(
			"audio_export_failed",
			"audio_export returned a WAV path that does not exist",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "existing WAV under audio_dir",
				"actual": wav_path,
			},
		))
		return
	var file := FileAccess.open(wav_path, FileAccess.READ)
	var size := file.get_length() if file != null else 0
	if size <= 0 or size > AUDIO_EXPORT_MAX_BYTES:
		_failures.append(_audio_failure(
			"audio_export_oversized",
			"audio_export wrote an empty or oversized WAV artifact",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "1..%d bytes" % AUDIO_EXPORT_MAX_BYTES,
				"actual": size,
			},
		))
		return
	if file == null or not _looks_like_wav(file.get_buffer(min(size, 12))):
		_failures.append(_audio_failure(
			"audio_export_invalid_return",
			"audio_export wrote a file that is not RIFF/WAVE data",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "RIFF/WAVE file",
				"actual": wav_path,
			},
		))
		return
	var metadata := result.duplicate(true)
	metadata.erase("wav")
	metadata.erase("path")
	metadata["source"] = "audio_export"
	metadata["target"] = target
	metadata["method"] = method_name
	_record_audio_artifact(name, safe_name, wav_path, metadata)


func _record_audio_artifact(name: String, safe_name: String, wav_path: String, metadata: Dictionary) -> void:
	var artifact := {
		"wav": wav_path,
		"name": name,
		"safe_name": safe_name,
	}
	if not metadata.is_empty():
		artifact["metadata"] = metadata.duplicate(true)
	_audio_artifacts[name] = artifact


func _audio_failure(failure_type: String, message: String, command: String, extra: Dictionary) -> Dictionary:
	var data := extra.duplicate(true)
	data["command"] = command
	data["artifacts"] = {
		"run_root": _artifact_run_root(),
		"audio_dir": _audio_artifact_dir(),
	}
	return _failure(failure_type, message, data)


func _audio_artifact_dir() -> String:
	return str(_artifacts.get("audio_dir", ""))


func _audio_wav_path(safe_name: String) -> String:
	if safe_name == "" or _audio_artifact_dir() == "":
		return ""
	return "%s/%s.wav" % [_audio_artifact_dir(), safe_name]


func _safe_audio_artifact_name(name: String) -> String:
	var safe := ""
	for index in range(name.length()):
		var character := name.substr(index, 1)
		if character.is_valid_identifier() or character.is_valid_int() or character in ["-", "_"]:
			safe += character
		else:
			safe += "_"
	return safe.strip_edges().trim_prefix(".")


func _is_path_under_audio_dir(path: String) -> bool:
	var audio_dir := _audio_artifact_dir().simplify_path()
	var simplified := path.simplify_path()
	return audio_dir != "" and (simplified == audio_dir or simplified.begins_with("%s/" % audio_dir))


func _looks_like_wav(bytes: PackedByteArray) -> bool:
	return bytes.size() >= 12 and bytes.slice(0, 4).get_string_from_ascii() == "RIFF" and bytes.slice(8, 12).get_string_from_ascii() == "WAVE"


func _is_json_compatible(value: Variant) -> bool:
	if value == null or value is bool or value is int or value is float or value is String:
		return true
	if value is Array:
		for item in value:
			if not _is_json_compatible(item):
				return false
		return true
	if value is Dictionary:
		for key in value.keys():
			if not (key is String or key is StringName):
				return false
			if not _is_json_compatible(value.get(key)):
				return false
		return true
	return false


func _snapshot_dir(snapshot_name: String) -> String:
	return "%s/%s" % [str(_artifacts.get("snapshots_dir", "")), snapshot_name]


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
	if _mode == "check":
		if str(_check.get("execution_mode", "")) == "batch":
			return {
				"ok": _failures.is_empty(),
				"execution_mode": "batch",
				"duration_ms": Time.get_ticks_msec() - _started_at_msec,
				"display_server": DisplayServer.get_name(),
				"failures": _failures.duplicate(true),
				"artifacts": _artifacts.duplicate(true),
			}
		if _check.has("id") and _check.has("kind") and _check.has("target"):
			var payload := {
				"ok": _failures.is_empty(),
				"id": str(_check.get("id", "")),
				"kind": str(_check.get("kind", "")),
				"target": str(_check.get("target", "")),
				"duration_ms": Time.get_ticks_msec() - _started_at_msec,
				"display_server": DisplayServer.get_name(),
				"runtime_errors": _runtime_errors.duplicate(true),
				"failures": _failures.duplicate(true),
				"artifacts": _artifacts.duplicate(true),
			}
			if not _check_data.is_empty():
				payload["data"] = _check_data.duplicate(true)
			if _has_time_control_evidence():
				payload["time_control"] = _time_control_evidence.duplicate(true)
			if _profile_enabled:
				payload["profile"] = _profile_payload()
			return payload
		var scene_payload := {
			"ok": _failures.is_empty(),
			"scene": str(_check.get("scene", "")),
			"duration_ms": Time.get_ticks_msec() - _started_at_msec,
			"startup_frames": max(int(_check.get("startup_frames", 0)), 0),
			"display_server": DisplayServer.get_name(),
			"runtime_errors": _runtime_errors.duplicate(true),
			"failures": _failures.duplicate(true),
			"artifacts": _artifacts.duplicate(true),
		}
		if _has_time_control_evidence():
			scene_payload["time_control"] = _time_control_evidence.duplicate(true)
		return scene_payload
	var payload := {
		"ok": _failures.is_empty(),
		"run_identity": {
			"run_id": str(_request_run.get("id", "")),
			"nonce": str(_request_run.get("nonce", "")),
			"started_at": str(_request_run.get("started_at", "")),
		},
		"scenario": str(_scenario.get("name", "")),
		"scene": str(_scenario.get("scene", "")),
		"duration_ms": Time.get_ticks_msec() - _started_at_msec,
		"display_server": DisplayServer.get_name(),
		"snapshots": _snapshots.duplicate(),
		"snapshot_artifacts": _snapshot_artifacts.duplicate(true),
		"failures": _failures.duplicate(),
		"artifacts": _artifacts.duplicate(),
	}
	if _asset_trial != null:
		payload["asset_trial"] = _asset_trial.payload()
	if _assertion_proof != null and _assertion_proof.enabled:
		payload["assertion_coverage"] = _assertion_proof.payload()
	if not _node_inspections.is_empty():
		payload["node_inspections"] = _node_inspections.duplicate(true)
	if not _state_records.is_empty():
		payload["state_records"] = _state_records.duplicate(true)
	if not _physics_overlays.is_empty():
		payload["physics_overlays"] = _physics_overlays.duplicate(true)
	if not _policy_drives.is_empty():
		payload["policy_drives"] = _policy_drives.duplicate(true)
	if not _audio_artifacts.is_empty():
		payload["audio_artifacts"] = _audio_artifacts.duplicate(true)
	payload["godot_args"] = _scenario.get("godot_args", _request_run.get("godot_args", [])).duplicate(true)
	payload["project_args"] = _scenario.get("project_args", _request_run.get("project_args", [])).duplicate(true)
	var timing := _timing.duplicate()
	timing["engine_total_ms"] = float(Time.get_ticks_msec() - _started_at_msec)
	payload["timing"] = timing
	if _profile_enabled:
		payload["profile"] = _profile_payload()
	if _soak != null:
		payload["soak"] = _soak.summary()
	if _has_time_control_evidence():
		payload["time_control"] = _time_control_evidence.duplicate(true)
	return payload


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
	if _runtime != null:
		var release_started_usec := _profile_now_usec()
		var failure_count := _failures.size()
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


func _batch_summary_entry(payload: Dictionary) -> Dictionary:
	return {
		"id": str(payload.get("id", "")),
		"kind": str(payload.get("kind", "")),
		"target": str(payload.get("target", "")),
		"ok": bool(payload.get("ok", false)),
		"duration_ms": payload.get("duration_ms", null),
		"runtime_error_count": payload.get("runtime_errors", []).size() if payload.get("runtime_errors", []) is Array else 0,
		"failure_count": payload.get("failures", []).size() if payload.get("failures", []) is Array else 0,
		"artifacts": payload.get("artifacts", {}),
	}


func _write_batch_summary(results: Array[Dictionary]) -> int:
	var summary_path := str(_artifacts.get("summary", ""))
	if summary_path == "":
		return OK
	return _write_json_file(summary_path, {
		"ok": _failures.is_empty(),
		"execution_mode": "batch",
		"test_count": results.size(),
		"results": results.duplicate(true),
		"failures": _failures.duplicate(true),
		"artifacts": _artifacts.duplicate(true),
	})


func _write_check_progress_event(event: String, check: Dictionary, artifacts: Dictionary, batch_artifacts: Dictionary, started_at: String, elapsed_ms, ok, failure_count) -> void:
	var payload := {
		"event": event,
		"id": str(check.get("id", "")),
		"kind": str(check.get("kind", "")),
		"target": str(check.get("target", "")),
		"started_at": started_at,
		"elapsed_ms": elapsed_ms,
		"ok": ok,
		"failure_count": failure_count,
		"artifacts": artifacts.duplicate(true),
	}
	_append_jsonl(str(batch_artifacts.get("progress", "")), payload)
	_append_jsonl(str(artifacts.get("progress", "")), payload)


func _write_scenario_heartbeat(event: String, operation: String, index: int, kind: String, command: String, context: Dictionary, started_at: String, started_usec: int, failure_count: int) -> void:
	var elapsed_ms := float(max(Time.get_ticks_usec() - started_usec, 0)) / 1000.0
	var payload := {
		"run_id": str(_request_run.get("id", "")),
		"nonce": str(_request_run.get("nonce", "")),
		"event": event,
		"operation": operation,
		"index": index,
		"kind": kind,
		"started_at": started_at,
		"elapsed_ms": elapsed_ms,
		"ok": _failures.size() == failure_count,
		"failure_count_delta": max(_failures.size() - failure_count, 0),
		"artifacts": _scenario_progress_artifacts(),
	}
	if operation == "assertion" and _active_assertion_source_id != "":
		payload["source_id"] = _active_assertion_source_id
	if command != "":
		payload["command"] = command
	for key in context.keys():
		payload[str(key)] = context.get(key)
	_append_jsonl(str(_artifacts.get("progress", _artifacts.get("progress_path", ""))), payload)


func _scenario_progress_artifacts() -> Dictionary:
	var artifacts := {}
	for key in ["run_root", "result_path", "logs_path", "progress", "progress_path", "snapshots_dir", "temporal_dir", "audio_dir"]:
		if _artifacts.has(key):
			artifacts[key] = str(_artifacts.get(key, ""))
	return artifacts


func _progress_now_usec() -> int:
	return Time.get_ticks_usec()


func _progress_context_for_operation(entry: Variant, fallback: String) -> Dictionary:
	if not entry is Dictionary:
		return {"scenario_operation": fallback}
	var operation := _operation_name(entry, fallback)
	var context := {"scenario_operation": operation}
	var value = entry.get(operation, null)
	if value is Dictionary:
		for key in _safe_progress_context(value).keys():
			context[key] = _safe_progress_context(value).get(key)
	elif value is String or value is StringName:
		if operation in ["snapshot"]:
			context["snapshot"] = str(value)
		elif operation in ["click", "node_inspect", "exists", "not_exists", "visible", "hidden", "enabled", "disabled", "focused", "inside_viewport"]:
			context["target"] = str(value)
	return context


func _safe_progress_context(params: Variant) -> Dictionary:
	var context := {}
	if not params is Dictionary:
		return context
	for key in ["target", "locator", "qa_id", "path", "node", "a", "b", "other", "snapshot"]:
		if params.has(key):
			context[key] = str(params.get(key, ""))
	return context


func _append_jsonl(path: String, payload: Dictionary) -> void:
	if path == "":
		return
	var file := FileAccess.open(path, FileAccess.READ_WRITE)
	if file == null:
		file = FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		return
	file.seek_end()
	file.store_line(JSON.stringify(payload))


func _progress_started_at() -> String:
	return "%dZ" % int(Time.get_unix_time_from_system())


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


func _artifact_run_root() -> String:
	return str(_artifacts.get("run_root", _artifacts.get("root", "")))


func _artifact_result_path() -> String:
	return str(_artifacts.get("result_path", _artifacts.get("result", "")))


func _capture_engine_settings() -> void:
	if _engine_settings_captured:
		return
	_original_max_fps = Engine.max_fps
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
