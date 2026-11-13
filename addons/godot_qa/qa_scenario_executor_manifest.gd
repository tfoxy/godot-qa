extends RefCounted


var _executor: WeakRef


func _init(executor: RefCounted) -> void:
	_executor = weakref(executor)


func _run_manifest_batch_check() -> int:
	var tests_value = _executor.get_ref()._check.get("tests", null)
	if not tests_value is Array:
		_executor.get_ref()._failures.append(_executor.get_ref()._failure(
			"batch_check_request_invalid",
			"batch check requests require a tests array",
			{
				"target": "check.tests",
				"expected": "array of manifest-backed check entries",
				"actual": tests_value,
			},
		))
		return await _executor.get_ref()._finalize_result()

	var tests: Array = tests_value
	var validation_failures: Array[Dictionary] = []
	for index in range(tests.size()):
		var entry = tests[index]
		if not entry is Dictionary:
			validation_failures.append(_executor.get_ref()._failure(
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
			validation_failures.append(_executor.get_ref()._failure(
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
			_executor.get_ref()._failures.append(validation_failure)
		return await _executor.get_ref()._finalize_result()

	var batch_artifacts: Dictionary = _executor.get_ref()._artifacts.duplicate(true)
	var batch_started_at: int = _executor.get_ref()._started_at_msec
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

		_executor.get_ref()._reset_check_runtime_state()
		_executor.get_ref()._started_at_msec = Time.get_ticks_msec()
		_executor.get_ref()._check = entry_check
		_executor.get_ref()._artifacts = (entry.get("artifacts") as Dictionary).duplicate(true)
		_executor.get_ref()._initialize_profile(entry)
		var progress_started_at: String = _executor.get_ref()._progress_started_at()
		var progress_started_msec := Time.get_ticks_msec()
		_executor.get_ref()._write_check_progress_event("start", entry_check, _executor.get_ref()._artifacts, batch_artifacts, progress_started_at, null, null, null)
		_executor.get_ref()._apply_time_control(_executor.get_ref()._check.get("time_control", {}), str(_executor.get_ref()._check.get("id", "")))
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
			_executor.get_ref()._failures.append(_executor.get_ref()._failure(
				"runtime_contamination_detected",
				"Engine time/physics settings changed during the test and were not restored",
				{
					"target": str(_executor.get_ref()._check.get("id", "")),
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
		var payload_or_error: Dictionary = await _executor.get_ref()._finalize_result_payload(true)
		if not bool(payload_or_error.get("write_ok", true)):
			had_artifact_write_failure = true
			var write_failure: Dictionary = _executor.get_ref()._failure(
				"result_write_failed",
				"qa_runner could not write per-test result.json during batch execution",
				{
					"target": str(_executor.get_ref()._check.get("id", "")),
					"expected": "writable result artifact",
					"actual": payload_or_error.get("result_path", ""),
					"artifacts": _executor.get_ref()._artifacts.duplicate(true),
				},
			)
			_executor.get_ref()._failures.append(write_failure)
			aggregate_failures.append(write_failure)
		var payload: Dictionary = payload_or_error.get("payload", {})
		if payload.is_empty():
			payload = _executor.get_ref()._result_payload()
		var payload_failures = payload.get("failures", [])
		var payload_failure_count: int = payload_failures.size() if payload_failures is Array else 0
		_executor.get_ref()._write_check_progress_event(
			"finish",
			entry_check,
			_executor.get_ref()._artifacts,
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
		_executor.get_ref()._clear_bridge_runtime_errors()
		_executor.get_ref()._reset_check_runtime_state()

	_executor.get_ref()._artifacts = batch_artifacts
	_executor.get_ref()._started_at_msec = batch_started_at
	_executor.get_ref()._check = {
		"execution_mode": "batch",
		"tests": tests.duplicate(true),
	}
	_executor.get_ref()._failures = aggregate_failures
	var summary_error := _write_batch_summary(summary_entries)
	if summary_error != OK:
		_executor.get_ref()._failures.append(_executor.get_ref()._failure(
			"batch_summary_write_failed",
			"qa_runner could not write batch summary artifact",
			{
				"target": "batch.summary",
				"expected": "writable batch summary artifact",
				"actual": str(_executor.get_ref()._artifacts.get("summary", "")),
				"artifacts": _executor.get_ref()._artifacts.duplicate(true),
			},
		))
		return await _executor.get_ref()._finalize_result()

	print(JSON.stringify({
		"ok": _executor.get_ref()._failures.is_empty() and not had_artifact_write_failure,
		"execution_mode": "batch",
		"test_count": summary_entries.size(),
		"failures": _executor.get_ref()._failures.duplicate(true),
		"artifacts": _executor.get_ref()._artifacts.duplicate(true),
	}))
	return 0 if _executor.get_ref()._failures.is_empty() and not had_artifact_write_failure else 1

func _run_manifest_check() -> void:
	_executor.get_ref()._check_data = {}
	var test_id := str(_executor.get_ref()._check.get("id", ""))
	var kind := str(_executor.get_ref()._check.get("kind", ""))
	var target := str(_executor.get_ref()._check.get("target", ""))
	if test_id == "" or kind == "" or target == "":
		_executor.get_ref()._failures.append(_executor.get_ref()._failure(
			"check_request_invalid",
			"manifest-backed check requests require id, kind, and target",
			{
				"target": test_id if test_id != "" else "check",
				"expected": {
					"id": "non-empty string",
					"kind": "scene or script",
					"target": "non-empty string",
				},
				"actual": _executor.get_ref()._check.duplicate(true),
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

	_executor.get_ref()._failures.append(_executor.get_ref()._failure(
		"check_request_invalid",
		"manifest-backed check kind is unsupported",
		{
			"target": test_id,
			"expected": ["scene", "script", "script_compile"],
			"actual": kind,
		},
	))

func _run_manifest_scene_check(test_id: String, target: String) -> void:
	var scene_load_started: int = _executor.get_ref()._profile_now_usec()
	var scene_load_failure_count: int = _executor.get_ref()._failures.size()
	if not await _executor.get_ref()._load_scene_instance(target):
		_executor.get_ref()._record_profile_timing("scene_load", scene_load_started, scene_load_failure_count, {"target": test_id})
		return
	_executor.get_ref()._record_profile_timing("scene_load", scene_load_started, scene_load_failure_count, {"target": test_id})

	var initial_frames_started: int = _executor.get_ref()._profile_now_usec()
	var initial_frame_failure_count: int = _executor.get_ref()._failures.size()
	await _executor.get_ref()._tree.process_frame
	_executor.get_ref()._sample_profile_frame()
	RenderingServer.force_draw(false)
	await _executor.get_ref()._tree.process_frame
	_executor.get_ref()._sample_profile_frame()
	_executor.get_ref()._record_profile_timing("initial_rendered_frames", initial_frames_started, initial_frame_failure_count, {
		"target": test_id,
		"frames": 2,
	})

	var startup_frames := max(int(_executor.get_ref()._check.get("startup_frames", 0)), 0)
	var startup_started: int = _executor.get_ref()._profile_now_usec()
	var startup_failure_count: int = _executor.get_ref()._failures.size()
	for _frame_index in range(startup_frames):
		await _executor.get_ref()._tree.process_frame
		_executor.get_ref()._sample_profile_frame()
	_executor.get_ref()._record_profile_timing("startup_frames", startup_started, startup_failure_count, {
		"target": test_id,
		"frames": startup_frames,
	})

	var runtime_scan_started: int = _executor.get_ref()._profile_now_usec()
	var runtime_scan_failure_count: int = _executor.get_ref()._failures.size()
	if _executor.get_ref()._failures.is_empty() and not _executor.get_ref()._runtime_errors.is_empty():
		_executor.get_ref()._failures.append(_executor.get_ref()._failure(
			"runtime_errors_present",
			"runtime errors were recorded during startup",
			{
				"target": test_id,
				"expected": [],
				"actual": _executor.get_ref()._runtime_errors.duplicate(true),
			},
		))
	_executor.get_ref()._record_profile_timing("runtime_error_scan", runtime_scan_started, runtime_scan_failure_count, {"target": test_id})

func _run_manifest_script_check(test_id: String, target: String) -> void:
	var script_load_started: int = _executor.get_ref()._profile_now_usec()
	var script_load_failure_count: int = _executor.get_ref()._failures.size()
	var script_resource := load(target)
	if script_resource == null or not script_resource is GDScript:
		_executor.get_ref()._failures.append(_executor.get_ref()._failure(
			"script_test_load_failed",
			"script test could not be loaded",
			{
				"target": test_id,
				"expected": "loadable GDScript resource",
				"actual": target,
			},
		))
		_executor.get_ref()._record_profile_timing("script_load", script_load_started, script_load_failure_count, {"target": test_id})
		return
	_executor.get_ref()._record_profile_timing("script_load", script_load_started, script_load_failure_count, {"target": test_id})

	var instantiate_started: int = _executor.get_ref()._profile_now_usec()
	var instantiate_failure_count: int = _executor.get_ref()._failures.size()
	var script_instance: Variant = (script_resource as GDScript).new()
	script_resource = null
	if script_instance == null:
		_executor.get_ref()._failures.append(_executor.get_ref()._failure(
			"script_test_instantiate_failed",
			"script test could not be instantiated",
			{
				"target": test_id,
				"expected": "instantiated script object",
				"actual": "null",
			},
		))
		_executor.get_ref()._record_profile_timing("script_instantiate", instantiate_started, instantiate_failure_count, {"target": test_id})
		return
	_executor.get_ref()._record_profile_timing("script_instantiate", instantiate_started, instantiate_failure_count, {"target": test_id})
	if not script_instance.has_method("run_qa_test"):
		_executor.get_ref()._failures.append(_executor.get_ref()._failure(
			"script_test_method_missing",
			"script test must define run_qa_test()",
			{
				"target": test_id,
				"expected": "script instance with run_qa_test()",
				"actual": target,
			},
		))
		if script_instance is Node:
			var missing_teardown_started: int = _executor.get_ref()._profile_now_usec()
			var missing_teardown_failure_count: int = _executor.get_ref()._failures.size()
			(script_instance as Node).free()
			_executor.get_ref()._record_profile_timing("script_teardown", missing_teardown_started, missing_teardown_failure_count, {"target": test_id})
		script_instance = null
		return

	await _executor.get_ref()._tree.process_frame
	_executor.get_ref()._sample_profile_frame()
	var run_started: int = _executor.get_ref()._profile_now_usec()
	var run_failure_count: int = _executor.get_ref()._failures.size()
	var raw_result: Variant = await script_instance.call("run_qa_test")
	_executor.get_ref()._record_profile_timing("run_qa_test", run_started, run_failure_count, {"target": test_id})
	await _executor.get_ref()._tree.process_frame
	_executor.get_ref()._sample_profile_frame()
	if script_instance is Node:
		var teardown_started: int = _executor.get_ref()._profile_now_usec()
		var teardown_failure_count: int = _executor.get_ref()._failures.size()
		(script_instance as Node).free()
		_executor.get_ref()._record_profile_timing("script_teardown", teardown_started, teardown_failure_count, {"target": test_id})
	script_instance = null
	if not raw_result is Dictionary:
		var validation_started: int = _executor.get_ref()._profile_now_usec()
		var validation_failure_count: int = _executor.get_ref()._failures.size()
		_executor.get_ref()._failures.append(_executor.get_ref()._failure(
			"script_test_result_invalid",
			"script test must return a dictionary",
			{
				"target": test_id,
				"expected": "dictionary with ok and failures",
				"actual": raw_result,
			},
		))
		_executor.get_ref()._record_profile_timing("result_validation", validation_started, validation_failure_count, {"target": test_id})
		return

	var validation_started: int = _executor.get_ref()._profile_now_usec()
	var validation_failure_count: int = _executor.get_ref()._failures.size()
	var result_dict: Dictionary = raw_result
	var ok_value = result_dict.get("ok", null)
	if not ok_value is bool:
		_executor.get_ref()._failures.append(_executor.get_ref()._failure(
			"script_test_result_invalid",
			"script test result field ok must be a boolean",
			{
				"target": test_id,
				"expected": "boolean",
				"actual": ok_value,
			},
		))
		_executor.get_ref()._record_profile_timing("result_validation", validation_started, validation_failure_count, {"target": test_id})
		return
	var failures_value = result_dict.get("failures", null)
	if not failures_value is Array:
		_executor.get_ref()._failures.append(_executor.get_ref()._failure(
			"script_test_result_invalid",
			"script test result field failures must be an array",
			{
				"target": test_id,
				"expected": "array of dictionaries",
				"actual": failures_value,
			},
		))
		_executor.get_ref()._record_profile_timing("result_validation", validation_started, validation_failure_count, {"target": test_id})
		return

	var data_copy_started: int = _executor.get_ref()._profile_now_usec()
	var data_copy_failure_count: int = _executor.get_ref()._failures.size()
	for failure_entry in failures_value:
		if not failure_entry is Dictionary:
			_executor.get_ref()._failures.append(_executor.get_ref()._failure(
				"script_test_result_invalid",
				"script test failure entries must be dictionaries",
				{
					"target": test_id,
					"expected": "dictionary failure entries",
					"actual": failure_entry,
				},
			))
			_executor.get_ref()._record_profile_timing("result_validation", validation_started, validation_failure_count, {"target": test_id})
			return
		_executor.get_ref()._failures.append(_normalize_script_test_failure(test_id, failure_entry))
	_executor.get_ref()._record_profile_timing("result_validation", validation_started, validation_failure_count, {"target": test_id})
	_executor.get_ref()._record_profile_timing("data_copy", data_copy_started, data_copy_failure_count, {
		"target": test_id,
		"failure_count": failures_value.size(),
	})

	if result_dict.has("data"):
		var data_value = result_dict.get("data")
		if data_value is Dictionary:
			_executor.get_ref()._check_data = (data_value as Dictionary).duplicate(true)
		else:
			_executor.get_ref()._failures.append(_executor.get_ref()._failure(
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
		_executor.get_ref()._failures.append(_executor.get_ref()._failure(
			"script_test_failed",
			"script test reported failure without failure details",
			{
				"target": test_id,
				"expected": "at least one failure entry when ok is false",
				"actual": [],
			},
		))

func _run_script_compile_check(test_id: String, target: String) -> void:
	var script_load_started: int = _executor.get_ref()._profile_now_usec()
	var script_load_failure_count: int = _executor.get_ref()._failures.size()
	var script_resource := load(target)
	if script_resource == null or not script_resource is GDScript:
		_executor.get_ref()._failures.append(_executor.get_ref()._failure(
			"script_load_failed",
			"script could not be loaded as GDScript",
			{
				"target": target,
				"expected": "loadable GDScript resource",
				"actual": target,
			},
		))
		_executor.get_ref()._record_profile_timing("script_load", script_load_started, script_load_failure_count, {"target": test_id})
		return
	_executor.get_ref()._check_data = {
		"status": "passed",
		"validated_resource": target,
	}
	script_resource = null
	_executor.get_ref()._record_profile_timing("script_load", script_load_started, script_load_failure_count, {"target": test_id})

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
		normalized["artifacts"] = {"run_root": _executor.get_ref()._artifact_run_root()}
	return normalized

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
	var summary_path := str(_executor.get_ref()._artifacts.get("summary", ""))
	if summary_path == "":
		return OK
	return _executor.get_ref()._write_json_file(summary_path, {
		"ok": _executor.get_ref()._failures.is_empty(),
		"execution_mode": "batch",
		"test_count": results.size(),
		"results": results.duplicate(true),
		"failures": _executor.get_ref()._failures.duplicate(true),
		"artifacts": _executor.get_ref()._artifacts.duplicate(true),
	})
