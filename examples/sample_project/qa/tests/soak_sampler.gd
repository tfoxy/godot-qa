extends SceneTree

const QaSoak = preload("res://addons/godot_qa/qa_soak.gd")
var epoch := 1
var tick := 0
var active := true
var phase := "running"
var nodes := 4
var omit_tick := false

func _initialize() -> void:
	call_deferred("_run")

func qa_state() -> Dictionary:
	var state := {"epoch": epoch, "tick": tick, "active": active, "phase": phase, "counts": {"nodes": nodes}}
	if omit_tick:
		state.erase("tick")
	return state

func has_failure(result: Dictionary, kind: String) -> bool:
	for violation in result.get("first_failure", {}).get("violations", []):
		if str(violation.get("type", "")) == kind:
			return true
	return false

func _run() -> void:
	var base := ProjectSettings.globalize_path("user://qa_soak_tests").path_join(str(Time.get_ticks_usec()))
	var spec := {"version": 1, "duration_seconds": 1.0, "window_seconds": 1.0, "state": {"provider": "fixture", "epoch": "epoch", "tick": "tick", "active": "active", "phase": "phase"}, "gauges": [{"name": "nodes", "path": "counts.nodes", "max": 10, "final_max": 10}], "budgets": {"frame_max_ms": 150, "min_active_ticks_per_second": 55}}
	var json_spec: Variant = JSON.parse_string('{"version":1,"duration_seconds":1,"window_seconds":1}')
	var json_started: Dictionary = QaSoak.new().start(json_spec, null, base.path_join("json_version"), "json_version")
	if not bool(json_started.get("ok", false)):
		push_error("JSON-parsed version 1 was rejected: " + JSON.stringify(json_started))
		quit(1)
		return
	for invalid_spec in [{"version": 1.5}, {"version": "1"}, {"version": true}, {"version": 1, "unknown": true}]:
		var rejected: Dictionary = QaSoak.new().start(invalid_spec, null, base.path_join("invalid"), "invalid")
		if bool(rejected.get("ok", true)) or str(rejected.get("type", "")) != "soak_spec_invalid":
			push_error("invalid soak root was accepted: " + JSON.stringify(invalid_spec))
			quit(1)
			return
	var warm_spec := {"version": 1, "warmup_seconds": 1.0, "duration_seconds": 1.0, "window_seconds": 1.0}
	var warm := QaSoak.new()
	var initial_process := Engine.get_process_frames()
	var warm_started: Dictionary = warm.start(warm_spec, null, base.path_join("warmup"), "warmup")
	if not bool(warm_started.get("ok", false)):
		push_error("warmup soak start failed: " + JSON.stringify(warm_started))
		quit(1)
		return
	var warmup_samples := 0
	for i in 5:
		await process_frame
		warm.sample()
		warmup_samples += 1
	if warm.measurement_started or int(warm.summary().get("engine_frames", {}).get("process", -1)) != 0:
		push_error("warmup advanced measurement early")
		quit(1)
		return
	while Time.get_ticks_usec() < warm.measured_start_usec:
		await process_frame
		warm.sample()
	while warm.status == "running":
		await process_frame
		warm.sample()
	var warm_result: Dictionary = warm.summary()
	var warm_window: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(base.path_join("warmup/windows.jsonl")).get_slice("\n", 0))
	var measured_process := int(warm_result.get("engine_frames", {}).get("process", -1))
	var warmup_process := int(warm_result.get("warmup_engine_frames", {}).get("process", -1))
	if not bool(warm_result.get("ok", false)) or warmup_process < 5 or measured_process <= 0 or int(warm_window.get("engine_frames", {}).get("process", -1)) > measured_process or warmup_process + measured_process != Engine.get_process_frames() - initial_process:
		push_error("warmup engine frame accounting failed: " + JSON.stringify(warm_result))
		quit(1)
		return
	var positive := QaSoak.new()
	var started: Dictionary = positive.start(spec, null, base.path_join("positive"), "positive", "", Callable(self, "qa_state"))
	if not bool(started.get("ok", false)):
		push_error("positive soak start failed: " + JSON.stringify(started))
		quit(1)
		return
	var count := 0
	while positive.status == "running":
		await process_frame
		count += 1
		if count == 30:
			epoch += 1
			tick = 0
		if count >= 50 and count < 70:
			active = false
			phase = "paused"
		else:
			active = true
			phase = "post_reset" if epoch > 1 else "running"
		if active:
			tick += 1
		positive.sample()
	if not bool(positive.summary().get("ok", false)) or int(positive.summary().get("windows_completed", 0)) != 1:
		push_error("positive soak failed: " + JSON.stringify(positive.summary()))
		quit(1)
		return
	var positive_window: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(base.path_join("positive/windows.jsonl")).get_slice("\n", 0))
	if int(positive_window.get("epoch_resets", 0)) != 1 or not positive_window.get("phase_counts", {}).has("paused"):
		push_error("reset/pause evidence missing: " + JSON.stringify(positive_window))
		quit(1)
		return
	var negative := QaSoak.new()
	negative.start(spec, null, base.path_join("negative"), "negative", "", Callable(self, "qa_state"))
	count = 0
	while negative.status == "running":
		await process_frame
		count += 1
		if count == 5:
			OS.delay_msec(170)
			nodes = 20
		# Deliberately stall tick progression while active.
		negative.sample()
	var result: Dictionary = negative.summary()
	if bool(result.get("ok", true)) or (result.get("first_failure", {}) as Dictionary).is_empty() or int(result.get("windows_completed", 0)) != 1:
		push_error("negative soak did not fail: " + JSON.stringify(result))
		quit(1)
		return
	var negative_window: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(base.path_join("negative/windows.jsonl")).get_slice("\n", 0))
	var kinds := {}
	for violation in negative_window.get("violations", []):
		kinds[str(violation.get("type", ""))] = true
	if not kinds.has("soak_frame_max") or not kinds.has("soak_tick_rate") or not kinds.has("soak_gauge_max") or not (result.get("first_failure", {}) as Dictionary).has("window"):
		push_error("spike/stall/leak evidence missing: " + JSON.stringify(result))
		quit(1)
		return
	var partial_spec := spec.duplicate(true)
	partial_spec["duration_seconds"] = 2.0
	var partial := QaSoak.new()
	partial.start(partial_spec, null, base.path_join("partial"), "partial", "", Callable(self, "qa_state"))
	for i in 10:
		await process_frame
		partial.sample()
	var stopped: Dictionary = partial.stop()
	if bool(stopped.get("complete", true)) or int(stopped.get("partial_window", {}).get("frame_sample_count", 0)) < 1:
		push_error("partial evidence missing: " + JSON.stringify(stopped))
		quit(1)
		return
	var missing := QaSoak.new()
	missing.start(spec, null, base.path_join("missing"), "missing", "", Callable(self, "qa_state"))
	omit_tick = true
	missing.sample()
	omit_tick = false
	if not has_failure(missing.summary(), "soak_state_invalid"):
		push_error("missing counter did not fail: " + JSON.stringify(missing.summary()))
		quit(1)
		return
	var regressed := QaSoak.new()
	tick = 10
	regressed.start(spec, null, base.path_join("regressed"), "regressed", "", Callable(self, "qa_state"))
	regressed.sample()
	tick = 9
	regressed.sample()
	regressed.stop()
	if not has_failure(regressed.summary(), "soak_tick_regressed"):
		push_error("undeclared tick regression did not fail: " + JSON.stringify(regressed.summary()))
		quit(1)
		return
	var overflow := QaSoak.new()
	overflow.start(spec, null, base.path_join("overflow"), "overflow", "", Callable(self, "qa_state"))
	overflow.frame_values.resize(QaSoak.MAX_INTERVALS_PER_WINDOW)
	while overflow.status == "running":
		await process_frame
		overflow.sample()
	if not has_failure(overflow.summary(), "soak_frame_overflow"):
		push_error("frame interval overflow did not fail: " + JSON.stringify(overflow.summary()))
		quit(1)
		return
	var retention_spec := spec.duplicate(true)
	retention_spec["gauges"] = [{"name": "nodes", "path": "counts.nodes", "final_max": 10}]
	retention_spec["budgets"] = {}
	var retained := QaSoak.new()
	nodes = 20
	retained.start(retention_spec, null, base.path_join("retained"), "retained", "", Callable(self, "qa_state"))
	while retained.status == "running":
		await process_frame
		tick += 1
		retained.sample()
	if not has_failure(retained.summary(), "soak_gauge_final"):
		push_error("final-only retention did not fail: " + JSON.stringify(retained.summary()))
		quit(1)
		return
	var stalled := QaSoak.new()
	nodes = 4
	active = true
	stalled.start(retention_spec, null, base.path_join("stalled"), "stalled", "", Callable(self, "qa_state"))
	while stalled.status == "running":
		await process_frame
		stalled.sample()
	if not has_failure(stalled.summary(), "soak_active_stall"):
		push_error("no-budget active stall did not fail: " + JSON.stringify(stalled.summary()))
		quit(1)
		return
	print("SOAK_SAMPLER_TEST_PASS")
	quit(0)
