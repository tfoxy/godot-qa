extends RefCounted

# One bounded wall-clock sampler shared by scenario and live profiling.
const MAX_INTERVALS_PER_WINDOW := 65536
const MONITORS := {
	"TIME_FPS": Performance.TIME_FPS,
	"TIME_PROCESS": Performance.TIME_PROCESS,
	"TIME_PHYSICS_PROCESS": Performance.TIME_PHYSICS_PROCESS,
	"OBJECT_NODE_COUNT": Performance.OBJECT_NODE_COUNT,
	"MEMORY_STATIC": Performance.MEMORY_STATIC,
}

var spec: Dictionary = {}
var runtime: RefCounted
var output_dir := ""
var run_id := ""
var status := "idle"
var failure: Dictionary = {}
var first_failure: Dictionary = {}
var worst_window: Dictionary = {}
var worst_windows: Dictionary = {}
var windows: Array[Dictionary] = []
var started_usec := 0
var finished_usec := 0
var measured_start_usec := 0
var next_end_usec := 0
var last_usec := 0
var last_epoch: Variant = null
var last_tick := 0
var last_active := false
var window_active_usec := 0
var window_ticks := 0
var window_resets := 0
var frame_values: Array[float] = []
var frame_overflow := false
var largest_gap_ms := 0.0
var gauge_peaks: Dictionary = {}
var gauge_final: Dictionary = {}
var gauge_total_peaks: Dictionary = {}
var window_monitor_samples: Array[Dictionary] = []
var omitted_monitor_samples := 0
var phase_counts: Dictionary = {}
var first_phase := ""
var last_phase := ""
var process_start := 0
var physics_start := 0
var drawn_start := 0
var process_total_start := 0
var physics_total_start := 0
var drawn_total_start := 0
var measured_process_start := 0
var measured_physics_start := 0
var measured_drawn_start := 0
var measurement_started := false
var warmup_engine_frames: Dictionary = {"process": 0, "physics": 0, "drawn": 0}
var final_engine_frames: Dictionary = {}
var frame_index := 0
var progress_path := ""
var provider: Callable
var provenance: Dictionary = {}


func start(config: Dictionary, state_runtime: RefCounted, artifact_dir: String, id: String, progress: String = "", state_provider: Callable = Callable(), environment: Dictionary = {}) -> Dictionary:
	var validation := _validate(config, state_runtime, state_provider)
	if not bool(validation.get("ok", false)):
		return validation
	spec = config.duplicate(true)
	runtime = state_runtime
	provider = state_provider
	provenance = environment.duplicate(true)
	output_dir = artifact_dir
	run_id = id
	progress_path = progress
	var err := DirAccess.make_dir_recursive_absolute(output_dir)
	if err != OK:
		return {"ok": false, "type": "artifact_write_failed", "path": output_dir, "error": err}
	started_usec = Time.get_ticks_usec()
	measured_start_usec = started_usec + int(float(spec.get("warmup_seconds", 0.0)) * 1000000.0)
	next_end_usec = measured_start_usec + int(float(spec.get("window_seconds", 5.0)) * 1000000.0)
	last_usec = started_usec
	process_start = Engine.get_process_frames()
	physics_start = Engine.get_physics_frames()
	drawn_start = Engine.get_frames_drawn()
	process_total_start = process_start
	physics_total_start = physics_start
	drawn_total_start = drawn_start
	measured_process_start = process_start
	measured_physics_start = physics_start
	measured_drawn_start = drawn_start
	measurement_started = measured_start_usec == started_usec
	warmup_engine_frames = {"process": 0, "physics": 0, "drawn": 0}
	status = "running"
	_write_summary()
	return {"ok": true, "run_id": run_id, "summary_path": output_dir.path_join("soak.json"), "windows_path": output_dir.path_join("windows.jsonl")}


func sample() -> void:
	if status != "running":
		return
	if not is_equal_approx(Engine.time_scale, 1.0):
		_fail("soak_pacing_changed", {"time_scale": Engine.time_scale})
		_finish("failed")
		return
	var now := Time.get_ticks_usec()
	if now < measured_start_usec:
		last_usec = now
		return
	if not measurement_started:
		# The first measured sample owns the boundary. No warmup engine frame
		# may enter a measured window or the summary's measured total.
		warmup_engine_frames = _engine_frames_since(process_total_start, physics_total_start, drawn_total_start)
		measured_process_start = Engine.get_process_frames()
		measured_physics_start = Engine.get_physics_frames()
		measured_drawn_start = Engine.get_frames_drawn()
		process_start = measured_process_start
		physics_start = measured_physics_start
		drawn_start = measured_drawn_start
		measurement_started = true
	if last_usec < measured_start_usec:
		last_usec = measured_start_usec
	var finish_usec := measured_start_usec + int(float(spec.get("duration_seconds", 600.0)) * 1000000.0)
	var sample_end: int = now
	var interval_usec: int = maxi(now - last_usec, 0)
	if interval_usec > 0:
		largest_gap_ms = maxf(largest_gap_ms, float(interval_usec) / 1000.0)
		if frame_values.size() >= MAX_INTERVALS_PER_WINDOW:
			frame_overflow = true
		else:
			frame_values.append(float(interval_usec) / 1000.0)
		if last_active:
			window_active_usec += maxi(mini(now, finish_usec) - last_usec, 0)
	last_usec = sample_end
	frame_index += 1
	var state := _read_state()
	if not bool(state.get("ok", false)):
		_fail("soak_state_invalid", state)
		_finish("failed")
		return
	if state.has("epoch"):
		var epoch: Variant = state.get("epoch")
		var tick := int(state.get("tick", 0))
		if last_epoch != null:
			if epoch != last_epoch:
				window_resets += 1
			elif tick < last_tick:
				_fail("soak_tick_regressed", {"epoch": epoch, "before": last_tick, "after": tick})
			elif last_active:
				window_ticks += tick - last_tick
		last_epoch = epoch
		last_tick = tick
		last_active = bool(state.get("active", false))
	var values: Dictionary = state.get("values", {})
	if state.has("phase"):
		var phase := str(state.get("phase"))
		if first_phase == "":
			first_phase = phase
		last_phase = phase
		if phase_counts.has(phase) or phase_counts.size() < 32:
			phase_counts[phase] = int(phase_counts.get(phase, 0)) + 1
		else:
			phase_counts["__other__"] = int(phase_counts.get("__other__", 0)) + 1
	for key in values:
		var value := float(values[key])
		gauge_final[key] = value
		gauge_peaks[key] = maxf(value, float(gauge_peaks.get(key, value)))
		gauge_total_peaks[key] = maxf(value, float(gauge_total_peaks.get(key, value)))
	if frame_index % int(spec.get("monitor_every_frames", 60)) == 0:
		var readings := {}
		for name in MONITORS:
			readings[name] = Performance.get_monitor(MONITORS[name])
		if window_monitor_samples.size() < 120:
			window_monitor_samples.append({"elapsed_usec": now - measured_start_usec, "values": readings})
		else:
			omitted_monitor_samples += 1
	while sample_end >= next_end_usec and next_end_usec <= finish_usec and status == "running":
		_close_window(next_end_usec)
		next_end_usec += int(float(spec.get("window_seconds", 5.0)) * 1000000.0)
	if now >= finish_usec and status == "running":
		# Duration must be an exact multiple of the window for a pass.
		if frame_values.size() > 0:
			_fail("soak_partial_window", {"sample_count": frame_values.size()})
		_finish("complete")


func stop() -> Dictionary:
	if status == "running":
		_finish("stopped")
	return summary()


func summary() -> Dictionary:
	return {
		"run_id": run_id, "status": status, "complete": status == "complete",
		"ok": status == "complete" and first_failure.is_empty(),
		"spec": spec.duplicate(true),
		"warmup_seconds": float(spec.get("warmup_seconds", 0.0)),
		"measured_seconds": clampf(float(maxi((finished_usec if finished_usec > 0 else Time.get_ticks_usec()) - measured_start_usec, 0)) / 1000000.0, 0.0, float(spec.get("duration_seconds", 600.0))),
		"windows_completed": windows.size(), "expected_windows": int(round(float(spec.get("duration_seconds", 600.0)) / float(spec.get("window_seconds", 5.0)))),
		"first_failure": first_failure, "worst_window": worst_window, "worst_windows": worst_windows,
		"partial_window": _partial_window(),
		"gauge_final": gauge_final, "gauge_highwater": gauge_total_peaks,
		"provenance": provenance,
		"elapsed_seconds": float(maxi((finished_usec if finished_usec > 0 else Time.get_ticks_usec()) - started_usec, 0)) / 1000000.0,
		"engine_frames": final_engine_frames if finished_usec > 0 else (_engine_frames_since(measured_process_start, measured_physics_start, measured_drawn_start) if measurement_started else {"process": 0, "physics": 0, "drawn": 0}),
		"warmup_engine_frames": warmup_engine_frames if measurement_started else _engine_frames_since(process_total_start, physics_total_start, drawn_total_start),
		"paths": {"summary": output_dir.path_join("soak.json"), "windows": output_dir.path_join("windows.jsonl")},
	}


func _partial_window() -> Dictionary:
	if frame_values.is_empty() and window_ticks == 0 and window_active_usec == 0:
		return {}
	var sorted := frame_values.duplicate()
	sorted.sort()
	var count := sorted.size()
	return {
		"index": windows.size(), "frame_sample_count": count,
		"frame_p95_ms": sorted[clampi(int(ceil(float(count) * 0.95)) - 1, 0, count - 1)] if count > 0 else 0.0,
		"frame_max_ms": sorted[count - 1] if count > 0 else 0.0,
		"largest_observed_gap_ms": largest_gap_ms,
		"active_seconds": float(window_active_usec) / 1000000.0,
		"ticks": window_ticks, "epoch_resets": window_resets,
		"phase_counts": phase_counts.duplicate(true), "first_phase": first_phase, "last_phase": last_phase,
		"gauge_highwater": gauge_peaks.duplicate(true), "gauge_final": gauge_final.duplicate(true),
		"frame_overflow": frame_overflow,
	}


func _read_state() -> Dictionary:
	var mapping: Dictionary = spec.get("state", {})
	if mapping.is_empty():
		return {"ok": true, "values": {}}
	var source: Variant
	if provider.is_valid():
		source = provider.call()
	else:
		var read: Dictionary = runtime.read_state_provider(str(mapping.get("provider", "")))
		if not bool(read.get("ok", false)):
			return read
		source = read.get("state")
	var epoch: Variant = _path(source, str(mapping.get("epoch", "")))
	var tick: Variant = _path(source, str(mapping.get("tick", "")))
	var active: Variant = _path(source, str(mapping.get("active", "")))
	if not (epoch is int or epoch is String) or not tick is int or int(tick) < 0 or not active is bool:
		return {"ok": false, "expected": "epoch, nonnegative tick, bool active", "actual": {"epoch": epoch, "tick": tick, "active": active}}
	var values := {}
	var phase: Variant = null
	if mapping.has("phase"):
		phase = _path(source, str(mapping.get("phase")))
		if not phase is String:
			return {"ok": false, "expected": "string phase", "actual": phase}
	for gauge in spec.get("gauges", []):
		var value: Variant = _path(source, str(gauge.get("path", "")))
		if not (value is int or value is float) or not is_finite(float(value)):
			return {"ok": false, "expected": "finite numeric gauge", "actual": {"name": gauge.get("name"), "value": value}}
		values[str(gauge.get("name"))] = value
	var result := {"ok": true, "epoch": epoch, "tick": tick, "active": active, "values": values}
	if phase != null:
		result["phase"] = phase
	return result


func _path(source: Variant, path: String) -> Variant:
	var value: Variant = source
	for part in path.split("."):
		if not value is Dictionary or not (value as Dictionary).has(part):
			return null
		value = (value as Dictionary).get(part)
	return value


func _close_window(end_usec: int) -> void:
	var number := windows.size()
	var sorted := frame_values.duplicate()
	sorted.sort()
	var count := sorted.size()
	var p95 := float(sorted[clampi(int(ceil(float(count) * 0.95)) - 1, 0, maxi(count - 1, 0))]) if count > 0 else 0.0
	var maximum := float(sorted[count - 1]) if count > 0 else 0.0
	var active_seconds := float(window_active_usec) / 1000000.0
	var rate := float(window_ticks) / active_seconds if active_seconds > 0.0 else 0.0
	var row := {"index": number, "start_seconds": float(number) * float(spec.get("window_seconds", 5.0)), "end_seconds": float(end_usec - measured_start_usec) / 1000000.0, "frame_sample_count": count, "frame_p95_ms": p95, "frame_max_ms": maximum, "largest_observed_gap_ms": largest_gap_ms, "active_seconds": active_seconds, "ticks": window_ticks, "active_ticks_per_second": rate, "epoch_resets": window_resets, "phase_counts": phase_counts.duplicate(true), "first_phase": first_phase, "last_phase": last_phase, "gauge_highwater": gauge_peaks.duplicate(true), "gauge_final": gauge_final.duplicate(true), "engine_frames": {"process": Engine.get_process_frames() - process_start, "physics": Engine.get_physics_frames() - physics_start, "drawn": Engine.get_frames_drawn() - drawn_start}, "monitor_samples": window_monitor_samples.duplicate(true), "monitor_omitted_samples": omitted_monitor_samples, "monitor_cadence_frames": int(spec.get("monitor_every_frames", 60)), "monitor_provenance": "cached_godot_performance", "violations": []}
	var violations: Array = row["violations"]
	var budgets: Dictionary = spec.get("budgets", {})
	if frame_overflow:
		violations.append({"type": "soak_frame_overflow", "limit": MAX_INTERVALS_PER_WINDOW})
	if count == 0:
		violations.append({"type": "soak_no_frames"})
	if largest_gap_ms >= float(spec.get("window_seconds", 5.0)) * 1000.0:
		violations.append({"type": "soak_window_overrun", "gap_ms": largest_gap_ms})
	if budgets.has("frame_p95_ms") and p95 > float(budgets.get("frame_p95_ms")):
		violations.append({"type": "soak_frame_p95", "limit": budgets.get("frame_p95_ms"), "actual": p95})
	if budgets.has("frame_max_ms") and maximum > float(budgets.get("frame_max_ms")):
		violations.append({"type": "soak_frame_max", "limit": budgets.get("frame_max_ms"), "actual": maximum})
	if budgets.has("min_active_ticks_per_second") and active_seconds > 0.0 and rate < float(budgets.get("min_active_ticks_per_second")):
		violations.append({"type": "soak_tick_rate", "minimum": budgets.get("min_active_ticks_per_second"), "actual": rate})
	if not (spec.get("state", {}) as Dictionary).is_empty() and active_seconds >= 0.1 and window_ticks == 0:
		violations.append({"type": "soak_active_stall", "active_seconds": active_seconds})
	for gauge in spec.get("gauges", []):
		var name := str(gauge.get("name"))
		if gauge.has("max") and float(gauge_peaks.get(name, 0.0)) > float(gauge.get("max")):
			violations.append({"type": "soak_gauge_max", "name": name, "limit": gauge.get("max"), "actual": gauge_peaks.get(name)})
	if not violations.is_empty() and first_failure.is_empty():
		first_failure = {"window": number, "violations": violations.duplicate(true)}
	if worst_window.is_empty() or p95 > float(worst_window.get("frame_p95_ms", 0.0)):
		worst_window = {"index": number, "frame_p95_ms": p95, "frame_max_ms": maximum}
	_update_worst("frame_p95_ms", number, p95, true)
	_update_worst("frame_max_ms", number, maximum, true)
	if active_seconds >= 0.1:
		_update_worst("active_ticks_per_second", number, rate, false)
	for gauge in spec.get("gauges", []):
		var name := str(gauge.get("name"))
		_update_worst("gauge:" + name, number, float(gauge_peaks.get(name, 0.0)), true)
	windows.append(row)
	_append_jsonl(output_dir.path_join("windows.jsonl"), row)
	if progress_path != "":
		_append_jsonl(progress_path, {"event": "soak_window", "run_id": run_id, "index": number, "ok": violations.is_empty(), "summary": output_dir.path_join("soak.json")})
	frame_values.clear()
	frame_overflow = false
	largest_gap_ms = 0.0
	window_active_usec = 0
	window_ticks = 0
	window_resets = 0
	window_monitor_samples.clear()
	omitted_monitor_samples = 0
	phase_counts.clear()
	first_phase = ""
	last_phase = ""
	process_start = Engine.get_process_frames()
	physics_start = Engine.get_physics_frames()
	drawn_start = Engine.get_frames_drawn()
	gauge_peaks.clear()
	_write_summary()


func _update_worst(metric: String, index: int, value: float, higher_is_worse: bool) -> void:
	if not worst_windows.has(metric) or (value > float(worst_windows[metric].get("value", 0.0)) if higher_is_worse else value < float(worst_windows[metric].get("value", 0.0))):
		worst_windows[metric] = {"index": index, "value": value}


func _fail(kind: String, actual: Variant) -> void:
	failure = {"type": kind, "actual": actual}
	if first_failure.is_empty():
		first_failure = {"window": windows.size(), "violations": [failure]}


func _finish(reason: String) -> void:
	if reason == "complete":
		for gauge in spec.get("gauges", []):
			if gauge.has("final_max") and float(gauge_final.get(str(gauge.get("name")), 0.0)) > float(gauge.get("final_max")):
				_fail("soak_gauge_final", {"name": gauge.get("name"), "limit": gauge.get("final_max"), "actual": gauge_final.get(str(gauge.get("name")))})
	status = reason
	finished_usec = Time.get_ticks_usec()
	if not measurement_started:
		warmup_engine_frames = _engine_frames_since(process_total_start, physics_total_start, drawn_total_start)
	final_engine_frames = _engine_frames_since(measured_process_start, measured_physics_start, measured_drawn_start) if measurement_started else {"process": 0, "physics": 0, "drawn": 0}
	_write_summary()


func _engine_frames_since(process_baseline: int, physics_baseline: int, drawn_baseline: int) -> Dictionary:
	return {"process": Engine.get_process_frames() - process_baseline, "physics": Engine.get_physics_frames() - physics_baseline, "drawn": Engine.get_frames_drawn() - drawn_baseline}


func _write_summary() -> void:
	var file := FileAccess.open(output_dir.path_join("soak.json"), FileAccess.WRITE)
	if file != null:
		file.store_string(JSON.stringify(summary(), "  "))
		file.flush()
	else:
		_fail("artifact_write_failed", {"path": output_dir.path_join("soak.json")})


func _append_jsonl(path: String, row: Dictionary) -> void:
	var file := FileAccess.open(path, FileAccess.READ_WRITE) if FileAccess.file_exists(path) else FileAccess.open(path, FileAccess.WRITE_READ)
	if file == null:
		_fail("artifact_write_failed", {"path": path})
		return
	file.seek_end()
	file.store_line(JSON.stringify(row))
	file.flush()


func _validate(config: Dictionary, state_runtime: RefCounted, state_provider: Callable) -> Dictionary:
	for key in config:
		if key not in ["version", "warmup_seconds", "duration_seconds", "window_seconds", "state", "gauges", "budgets", "monitor_every_frames"]:
			return {"ok": false, "type": "soak_spec_invalid", "expected": "known soak specification field", "actual": key}
	var version: Variant = config.get("version")
	if not (version is int or version is float) or float(version) != 1.0:
		return {"ok": false, "type": "soak_spec_invalid", "expected": "version 1"}
	for key in ["duration_seconds", "warmup_seconds", "window_seconds"]:
		if config.has(key) and not (config[key] is int or config[key] is float):
			return {"ok": false, "type": "soak_spec_invalid", "expected": "numeric " + key}
	var duration := float(config.get("duration_seconds", 600.0))
	var warmup := float(config.get("warmup_seconds", 0.0))
	var window := float(config.get("window_seconds", 5.0))
	if not is_finite(duration) or not is_finite(warmup) or not is_finite(window) or duration <= 0.0 or duration > 3600.0 or warmup < 0.0 or warmup + duration > 3600.0 or window < 1.0 or window > 60.0 or not is_equal_approx(duration / window, round(duration / window)) or duration / window > 720.0:
		return {"ok": false, "type": "soak_spec_invalid", "expected": "0 <= warmup, 0 < duration, warmup+duration <= 3600, 1 <= window <= 60, exact window division and <=720 windows"}
	var mapping: Variant = config.get("state", {})
	if not mapping is Dictionary:
		return {"ok": false, "type": "soak_spec_invalid", "expected": "state object"}
	var state: Dictionary = mapping
	if not is_equal_approx(Engine.time_scale, 1.0) or OS.get_cmdline_args().has("--fixed-fps"):
		return {"ok": false, "type": "soak_realtime_required", "expected": "unit time scale and no --fixed-fps"}
	var cadence: Variant = config.get("monitor_every_frames", 60)
	if not (cadence is int or cadence is float) or not is_equal_approx(float(cadence), round(float(cadence))) or int(cadence) < 1 or int(cadence) > 3600:
		return {"ok": false, "type": "soak_spec_invalid", "expected": "monitor_every_frames integer 1..3600"}
	if not state.is_empty():
		for key in ["provider", "epoch", "tick", "active"]:
			if str(state.get(key, "")) == "":
				return {"ok": false, "type": "soak_spec_invalid", "expected": "state." + key}
		if state.has("phase") and str(state.get("phase", "")) == "":
			return {"ok": false, "type": "soak_spec_invalid", "expected": "nonempty state.phase path"}
		if state_runtime == null and not state_provider.is_valid():
			return {"ok": false, "type": "soak_spec_invalid", "expected": "runtime or read-only provider callable"}
	var budgets: Variant = config.get("budgets", {})
	if not budgets is Dictionary or (budgets as Dictionary).has("min_active_ticks_per_second") and state.is_empty():
		return {"ok": false, "type": "soak_spec_invalid", "expected": "valid budgets and state for throughput"}
	for key in (budgets as Dictionary):
		if not ["frame_p95_ms", "frame_max_ms", "min_active_ticks_per_second"].has(key) or not (budgets[key] is int or budgets[key] is float) or not is_finite(float(budgets[key])) or float(budgets[key]) <= 0.0:
			return {"ok": false, "type": "soak_spec_invalid", "expected": "positive known budgets", "actual": key}
	var gauges: Variant = config.get("gauges", [])
	if not gauges is Array or (gauges as Array).size() > 32 or not (gauges as Array).is_empty() and state.is_empty():
		return {"ok": false, "type": "soak_spec_invalid", "expected": "<=32 gauges with state"}
	var names := {}
	for gauge in gauges:
		if not gauge is Dictionary or str(gauge.get("name", "")) == "" or str(gauge.get("path", "")) == "":
			return {"ok": false, "type": "soak_spec_invalid", "expected": "named gauge with path"}
		if names.has(str(gauge.get("name"))):
			return {"ok": false, "type": "soak_spec_invalid", "expected": "unique gauge names"}
		names[str(gauge.get("name"))] = true
		for key in ["max", "final_max"]:
			if gauge.has(key) and (not (gauge[key] is int or gauge[key] is float) or not is_finite(float(gauge[key])) or float(gauge[key]) < 0.0):
				return {"ok": false, "type": "soak_spec_invalid", "expected": "nonnegative gauge budget"}
	return {"ok": true}
