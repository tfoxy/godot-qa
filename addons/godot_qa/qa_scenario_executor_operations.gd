extends RefCounted

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

var _executor: WeakRef


func _init(executor: RefCounted) -> void:
	_executor = weakref(executor)


func _run_assertion(assertion: Variant, assertion_index := -1, runtime_path := "") -> void:
	var started_usec: int = _executor.get_ref()._progress_now_usec()
	var failure_count: int = _executor.get_ref()._failures.size()
	_executor.get_ref()._active_assertion_source_id = _executor.get_ref()._assertion_proof.begin(runtime_path) if _executor.get_ref()._assertion_proof != null else ""
	var timing_extra := {
		"index": assertion_index,
		"kind": "assert",
	}
	var heartbeat_context: Dictionary = _executor.get_ref()._progress_context_for_operation(assertion, "assertion")
	var heartbeat_command := ""
	if assertion is Dictionary:
		var heartbeat_runtime_command := _runtime_command_for_assertion(assertion)
		if not heartbeat_runtime_command.is_empty():
			heartbeat_command = str(heartbeat_runtime_command.get("command", ""))
			heartbeat_context = heartbeat_runtime_command.get("progress_context", heartbeat_context)
	var heartbeat_started_at: String = _executor.get_ref()._progress_started_at()
	_executor.get_ref()._write_scenario_heartbeat("start", "assertion", assertion_index, "assert", heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)
	if not assertion is Dictionary or _assertion_operation_keys(assertion).size() != 1:
		_executor.get_ref()._failures.append(_executor.get_ref()._failure(
			"unsupported_assertion",
			"assertion must be an object with exactly one operation",
			{
				"target": "assertion",
				"expected": "object with exactly one supported operation",
				"actual": assertion,
			},
		))
		_executor.get_ref()._record_profile_timing("assertion", started_usec, failure_count, timing_extra)
		_executor.get_ref()._assertion_proof.finish(runtime_path, _executor.get_ref()._failures.size() == failure_count)
		_executor.get_ref()._write_scenario_heartbeat("finish", "assertion", assertion_index, "assert", heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)
		return

	var runtime_command := _runtime_command_for_assertion(assertion)
	if not runtime_command.is_empty():
		timing_extra["command"] = str(runtime_command.get("command", ""))
		heartbeat_context = runtime_command.get("progress_context", heartbeat_context)
		var runtime_started_usec: int = _executor.get_ref()._profile_now_usec()
		var runtime_result: Dictionary = await _executor.get_ref()._runtime.execute(
			str(runtime_command.get("command", "")),
			runtime_command.get("params", {}),
		)
		_executor.get_ref()._record_profile_timing(
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
			_executor.get_ref()._failures.append(_executor.get_ref()._runtime_error_to_failure(runtime_result.get("error", {})))
		_executor.get_ref()._record_profile_timing("assertion", started_usec, failure_count, timing_extra)
		_executor.get_ref()._assertion_proof.finish(runtime_path, _executor.get_ref()._failures.size() == failure_count)
		_executor.get_ref()._write_scenario_heartbeat("finish", "assertion", assertion_index, "assert", str(runtime_command.get("command", "")), heartbeat_context, heartbeat_started_at, started_usec, failure_count)
		return

	_executor.get_ref()._failures.append(_executor.get_ref()._failure(
		"unsupported_assertion",
		"M03 runner does not support this assertion yet",
		{
			"target": _operation_name(assertion, "assertion"),
			"expected": EXPANDED_ASSERTION_TYPES,
			"actual": assertion,
		},
	))
	_executor.get_ref()._record_profile_timing("assertion", started_usec, failure_count, timing_extra)
	_executor.get_ref()._assertion_proof.finish(runtime_path, _executor.get_ref()._failures.size() == failure_count)
	_executor.get_ref()._write_scenario_heartbeat("finish", "assertion", assertion_index, "assert", heartbeat_command, heartbeat_context, heartbeat_started_at, started_usec, failure_count)

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
		if _executor.get_ref()._frame_budget_exceeded():
			# Stop consuming frames; the step/assert boundary check owns the
			# authoritative scenario_frame_budget_exceeded failure.
			return {"ok": true, "frames": elapsed_frames, "stop_reason": "frame_budget"}
		var result: Dictionary = await _executor.get_ref()._runtime.execute("assert.state_matches", state_params.duplicate(true))
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
			await _executor.get_ref()._tree.process_frame
			_executor.get_ref()._sample_profile_frame()
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
		if _executor.get_ref()._frame_budget_exceeded():
			# Stop driving; the step/assert boundary check owns the
			# authoritative scenario_frame_budget_exceeded failure.
			stop_reason = "frame_budget"
			break
		if not until_params.is_empty():
			var until_result: Dictionary = await _executor.get_ref()._runtime.execute("assert.state_matches", until_params.duplicate(true))
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
		var tick_result: Dictionary = await _executor.get_ref()._runtime.execute("policy.drive_tick", {
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
			await _executor.get_ref()._tree.process_frame
			_executor.get_ref()._sample_profile_frame()
		elapsed_frames += step_frames
	# Always zero the driven devices' injected axes and release their pressed
	# buttons, even when the step is aborting on a failure.
	var release_result: Dictionary = await _executor.get_ref()._runtime.execute("policy.release_inputs", {"devices": device_list.duplicate()})
	if not bool(release_result.get("ok", false)) and failure.is_empty():
		var release_error = release_result.get("error", {})
		failure = {
			"type": str((release_error as Dictionary).get("type", "drive_policy_failed")) if release_error is Dictionary else "drive_policy_failed",
			"message": str((release_error as Dictionary).get("message", "drive_policy could not release driven inputs")) if release_error is Dictionary else "drive_policy could not release driven inputs",
			"data": (release_error as Dictionary).get("data", {}) if release_error is Dictionary else {"policy": policy},
		}
	_executor.get_ref()._policy_drives[str(step_index)] = {
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
	for key_step in ["input_key", "input_key_hold", "input_key_release"]:
		if step.has(key_step):
			var key_value = step.get(key_step)
			var key_params: Dictionary = {"key": str(key_value)}
			if key_value is Dictionary:
				key_params = {"key": str(key_value.get("key", ""))}
			if key_step != "input_key_release":
				key_params["frames"] = key_value.get("frames", 1) if key_value is Dictionary else 1
			return {"command": key_step.replace("input_", "input."), "params": key_params}
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
		var overlay_params: Dictionary = _executor.get_ref()._physics_overlay_step_params(step.get("physics_overlay", {}))
		return {
			"command": "physics.overlay.collect",
			"params": overlay_params,
			"progress_context": _executor.get_ref()._safe_progress_context(overlay_params),
		}
	if step.has("teleport_node"):
		var teleport_node: Dictionary = step.get("teleport_node", {})
		return {
			"command": "setup.teleport_node",
			"params": teleport_node,
			"progress_context": _executor.get_ref()._safe_progress_context(teleport_node),
		}
	if step.has("fixture_teleport"):
		var fixture_teleport: Dictionary = step.get("fixture_teleport", {})
		return {
			"command": "fixture.teleport_to_target",
			"params": fixture_teleport,
			"progress_context": _executor.get_ref()._safe_progress_context(fixture_teleport),
		}
	if step.has("spawn_scene"):
		var spawn_scene: Dictionary = step.get("spawn_scene", {})
		return {
			"command": "setup.spawn_scene",
			"params": spawn_scene,
			"progress_context": _executor.get_ref()._safe_progress_context(spawn_scene),
		}
	if step.has("set_property"):
		var set_property: Dictionary = step.get("set_property", {})
		return {
			"command": "setup.set_property",
			"params": set_property,
			"progress_context": _executor.get_ref()._safe_progress_context(set_property),
		}
	if step.has("call_method"):
		var call_method: Dictionary = step.get("call_method", {})
		return {
			"command": "setup.call_method",
			"params": call_method,
			"progress_context": _executor.get_ref()._safe_progress_context(call_method),
		}
	if step.has("fixture_call"):
		var fixture_call: Dictionary = step.get("fixture_call", {})
		return {
			"command": "fixture.call",
			"params": fixture_call,
			"progress_context": _executor.get_ref()._safe_progress_context(fixture_call),
		}
	return {}

func _temporal_params_with_artifacts(params: Dictionary) -> Dictionary:
	var merged := params.duplicate(true)
	if not _executor.get_ref()._scenario_temporal_sampling.is_empty():
		merged["temporal_sampling"] = _executor.get_ref()._scenario_temporal_sampling.duplicate(true)
	if params.get("temporal_sampling", null) is Dictionary:
		merged["temporal_sampling"] = (params.get("temporal_sampling", {}) as Dictionary).duplicate(true)
	var run_root: String = _executor.get_ref()._artifact_run_root()
	if run_root != "":
		merged["temporal_dir"] = "%s/temporal" % run_root
	return merged

func _runtime_command_for_assertion(assertion: Dictionary) -> Dictionary:
	var visual_dir := ""
	var run_root: String = _executor.get_ref()._artifact_run_root()
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
			"progress_context": _executor.get_ref()._safe_progress_context(no_overlap),
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
		if str(ra_params.get("snapshot", "")) == "" and _executor.get_ref()._last_snapshot_name != "":
			ra_params["snapshot"] = _executor.get_ref()._last_snapshot_name
		ra_params["visual_dir"] = visual_dir
		return {"command": "assert.renders_above", "params": ra_params}
	if assertion.has("renders_below"):
		var renders_below: Dictionary = assertion.get("renders_below", {})
		var rb_params := renders_below.duplicate(true)
		if rb_params.has("a") and not rb_params.has("target"):
			rb_params["target"] = rb_params.get("a")
		if rb_params.has("b") and not rb_params.has("other"):
			rb_params["other"] = rb_params.get("b")
		if str(rb_params.get("snapshot", "")) == "" and _executor.get_ref()._last_snapshot_name != "":
			rb_params["snapshot"] = _executor.get_ref()._last_snapshot_name
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
