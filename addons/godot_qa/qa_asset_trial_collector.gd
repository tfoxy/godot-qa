extends RefCounted

# Preloading the complete private runner validates the writer/probe helpers
# before baseline capability can authorize any source staging.
const TrialRunner = preload("res://addons/godot_qa/qa_asset_trial_runner.gd")
const Snapshot = preload("res://addons/godot_qa/qa_snapshot.gd")
const Probe = preload("res://addons/godot_qa/qa_asset_trial_probe.gd")
var _bindings: Array = []
var _captures: Dictionary = {}
var _audio_name := ""
var _audio_bus := ""
var _audio_frames := 0
var _audio_started_usec := 0
var _snapshot_facts: Dictionary = {}
var _snapshot_frame_callback: Callable
var _snapshot_frames := 0
var _snapshot_changed: Dictionary = {}


func _init(request: Dictionary = {}) -> void:
	if int(request.get("version", 0)) == 1 and request.get("bindings", []) is Array:
		_bindings = request.get("bindings", []).duplicate(true)


func payload() -> Dictionary:
	return {"version": 1, "capabilities": ["asset_trial_v1"], "backend": Probe.backend(), "captures": _captures.duplicate(true)}


func selected(kind: String, name: String) -> Array:
	var bindings: Array = []
	for value in _bindings:
		if value is Dictionary and value.get("evidence", {}) is Dictionary and str(value.get("evidence", {}).get(kind, "")) == name:
			bindings.append(value)
	return bindings


func snapshot_begin(runtime: RefCounted, name: String) -> void:
	_snapshot_facts.clear()
	_snapshot_changed.clear()
	_snapshot_frames = 0
	if selected("snapshot", name).is_empty():
		return
	_snapshot_frame_callback = func():
		_snapshot_frames += 1
		for binding in selected("snapshot", name):
			var id := str(binding.get("id", ""))
			var current := Probe.binding_facts(runtime, binding)
			if _snapshot_facts.has(id) and _snapshot_facts[id] != current:
				_snapshot_changed[id] = true
			_snapshot_facts[id] = current
	if not selected("snapshot", name).is_empty():
		RenderingServer.frame_post_draw.connect(_snapshot_frame_callback)


func snapshot(runtime: RefCounted, name: String, visual_dir: String) -> void:
	if _snapshot_frame_callback.is_valid() and RenderingServer.frame_post_draw.is_connected(_snapshot_frame_callback):
		RenderingServer.frame_post_draw.disconnect(_snapshot_frame_callback)
	_snapshot_frame_callback = Callable()
	var bindings: Array = []
	for binding in selected("snapshot", name):
		var facts: Dictionary = _snapshot_facts.get(str(binding.get("id", "")), Probe.binding_facts(runtime, binding))
		facts["rendered_frames"] = _snapshot_frames
		facts["binding_changed"] = bool(_snapshot_changed.get(str(binding.get("id", "")), false))
		# Existing visual probe measures actual rendered contribution by toggling
		# only the target, awaiting draws, and restoring its visibility.
		var visible: Dictionary = await runtime.execute("assert.visible_pixels", {"target": binding.get("runtime_target", ""), "min_count": 1, "snapshot": name, "visual_dir": visual_dir})
		var data: Dictionary = visible.get("data", {}) if bool(visible.get("ok", false)) else visible.get("error", {}).get("data", {})
		facts["visible_pixel_count"] = int(data.get("visible_pixel_count", 0))
		facts["visible_pixel_evidence"] = visible
		var isolated := isolated_texture_pixels(runtime, binding, visual_dir.path_join("binding-" + Probe.digest(str(binding.get("id", "binding")).to_utf8_buffer()).substr(0, 16)))
		facts["isolated_pixel_evidence"] = isolated
		facts["visible_pixel_count"] = mini(int(facts["visible_pixel_count"]), int(isolated.get("visible_pixel_count", 0)))
		var after := Probe.binding_facts(runtime, binding)
		facts["resource_after"] = after.get("resource", {})
		if facts.get("resource", {}) != after.get("resource", {}) or facts.get("instance_id") != after.get("instance_id"):
			facts["binding_changed"] = true
		bindings.append(facts)
	if not bindings.is_empty():
		_captures[name] = {"kind": "snapshot", "backend": Probe.backend(), "bindings": bindings}


# The shared visibility assertion hides a subtree across process frames. A
# second native draw check prevents children or an animated occluder from
# supplying the texture's proof. self_modulate affects this item's own drawing
# only; synchronous force_draw calls do not advance gameplay/process frames.
func isolated_texture_pixels(runtime: RefCounted, binding: Dictionary, directory: String) -> Dictionary:
	var resolution: Dictionary = runtime.resolve_target(str(binding.get("runtime_target", "")))
	var node := resolution.get("node") as CanvasItem
	if not bool(resolution.get("ok", false)) or node == null or not node.is_visible_in_tree():
		return {"visible_pixel_count": 0, "reason": "target_not_visible"}
	var window := node.get_tree().root
	var absolute := ProjectSettings.globalize_path(directory)
	if DirAccess.make_dir_recursive_absolute(absolute) != OK:
		return {"visible_pixel_count": 0, "reason": "artifact_directory_unavailable"}
	var before_path := absolute.path_join("own_draw_original.png")
	var hidden_path := absolute.path_join("own_draw_hidden.png")
	var original := node.self_modulate
	RenderingServer.force_draw(false, 0.0)
	var before_error := Snapshot.save_viewport_screenshot(window, before_path, false)
	node.self_modulate = Color(original.r, original.g, original.b, 0.0)
	RenderingServer.force_draw(false, 0.0)
	var hidden_error := Snapshot.save_viewport_screenshot(window, hidden_path, false)
	node.self_modulate = original
	RenderingServer.force_draw(false, 0.0)
	if before_error != OK or hidden_error != OK:
		return {"visible_pixel_count": 0, "reason": "rendered_pixels_unavailable", "before_error": before_error, "hidden_error": hidden_error}
	var before := Image.load_from_file(before_path)
	var hidden := Image.load_from_file(hidden_path)
	if before == null or hidden == null or before.get_size() != hidden.get_size():
		return {"visible_pixel_count": 0, "reason": "rendered_pixels_unavailable"}
	before.convert(Image.FORMAT_RGBA8)
	hidden.convert(Image.FORMAT_RGBA8)
	var left := before.get_data()
	var right := hidden.get_data()
	var count := 0
	for offset in range(0, left.size(), 4):
		if left[offset] != right[offset] or left[offset + 1] != right[offset + 1] or left[offset + 2] != right[offset + 2] or left[offset + 3] != right[offset + 3]:
			count += 1
	return {"visible_pixel_count": count, "mode": "synchronous_own_draw_delta", "artifacts": {"original": before_path, "target_own_draw_hidden": hidden_path}}


func audio_start(runtime: RefCounted, name: String, bus: String) -> void:
	_audio_name = name
	_audio_bus = bus
	_audio_frames = 0
	_audio_started_usec = Time.get_ticks_usec()
	var bindings: Array = []
	for binding in selected("audio", name):
		var facts := Probe.binding_facts(runtime, binding)
		facts["samples"] = []
		facts["playback_overlap"] = false
		facts["binding_changed"] = false
		facts["bus_changed"] = false
		facts["bus_matches"] = str(facts.get("bus", "")) == bus
		facts["observation_scope"] = "mixed_bus"
		bindings.append(facts)
	if not bindings.is_empty():
		_captures[name] = {"kind": "audio", "backend": Probe.backend(), "bus": bus, "bindings": bindings}
		audio_frame(runtime)


func audio_frame(runtime: RefCounted, final_sample: bool = false) -> void:
	if not _captures.has(_audio_name):
		return
	_audio_frames += 1
	var bindings := selected("audio", _audio_name)
	for index in bindings.size():
		var facts: Dictionary = _captures[_audio_name]["bindings"][index]
		var current := Probe.binding_facts(runtime, bindings[index], final_sample)
		var resource: Dictionary = current.get("resource", {})
		var previous: Dictionary = facts.get("resource_after", facts.get("resource", {}))
		if resource.get("instance_id") != previous.get("instance_id"):
			current = Probe.binding_facts(runtime, bindings[index])
			resource = current.get("resource", {})
		elif not final_sample:
			resource = previous.duplicate(true)
		var initial: Dictionary = facts.get("resource", {})
		if current.has("error") or current.get("instance_id") != facts.get("instance_id") or resource.get("instance_id") != initial.get("instance_id") or resource.get("fingerprint", {}) != initial.get("fingerprint", {}):
			facts["binding_changed"] = true
		if str(current.get("bus", "")) != _audio_bus:
			facts["bus_matches"] = false
		if current.get("bus") != facts.get("bus"):
			facts["bus_changed"] = true
		if bool(current.get("playing", false)) and not bool(current.get("stream_paused", false)) and str(current.get("bus", "")) == _audio_bus:
			facts["playback_overlap"] = true
		# Keep bounded transition evidence; counts still span every process_frame.
		var sample := {"resource": resource, "bus": current.get("bus", ""), "playing": current.get("playing", false), "stream_paused": current.get("stream_paused", false), "frame": _audio_frames}
		var samples: Array = facts["samples"]
		if samples.is_empty() or samples.back().get("resource") != resource or samples.back().get("playing") != sample["playing"] or samples.back().get("bus") != sample["bus"] or samples.back().get("stream_paused") != sample["stream_paused"]:
			if samples.size() < 256:
				samples.append(sample)
			else:
				facts["observation_overflow"] = true
		facts["resource_after"] = resource
		facts["observed_frames"] = _audio_frames


func audio_end(runtime: RefCounted) -> void:
	audio_frame(runtime, true)
	if _captures.has(_audio_name):
		_captures[_audio_name]["duration_seconds"] = float(Time.get_ticks_usec() - _audio_started_usec) / 1000000.0
	_audio_name = ""
