extends SceneTree

const Collector = preload("res://addons/godot_qa/qa_asset_trial_collector.gd")
const Runtime = preload("res://addons/godot_qa/qa_runtime.gd")
class AnimatedCover extends ColorRect:
	var animate := false
	var counter := 0
	func _process(_delta: float) -> void:
		if animate:
			counter += 1
			color = Color(float(counter % 251) / 251.0, 0.3, 0.7, 1)

var checks: Array = []
var failures: Array = []
var scene: Node2D
var sprite: Sprite2D
var cover: AnimatedCover
var player: AudioStreamPlayer
var runtime: RefCounted


func check(name: String, condition: bool, actual: Variant) -> void:
	checks.append(name)
	if not condition:
		failures.append({"name": name, "actual": actual})


func request(kind: String, name: String, target: String, property: String) -> Dictionary:
	return {"version": 1, "bindings": [{"id": "binding", "runtime_target": target, "property": property, "evidence": {kind: name}}]}


func snapshot(name: String, rebound: bool = false) -> Dictionary:
	var collector := Collector.new(request("snapshot", name, "qa:sprite", "texture"))
	collector.snapshot_begin(runtime, name)
	await process_frame
	await RenderingServer.frame_post_draw
	if rebound:
		sprite.texture = sprite.texture.duplicate()
		await process_frame
		await RenderingServer.frame_post_draw
	await collector.snapshot(runtime, name, ProjectSettings.globalize_path("res://.qa/visual/" + name))
	return collector.payload().captures[name].bindings[0]


func audio_start(name: String, bus: String = "Master") -> RefCounted:
	var collector := Collector.new(request("audio", name, "qa:player", "stream"))
	collector.audio_start(runtime, name, bus)
	return collector


func audio_facts(collector: RefCounted, name: String) -> Dictionary:
	collector.audio_end(runtime)
	return collector.payload().captures[name].bindings[0]


func _initialize() -> void:
	call_deferred("run")


func run() -> void:
	scene = Node2D.new()
	scene.name = "Evidence"
	get_root().add_child(scene)
	var background_marker := ColorRect.new()
	background_marker.size = Vector2(8, 96)
	background_marker.color = Color(0.2, 0.3, 0.4, 1)
	background_marker.z_index = -10
	scene.add_child(background_marker)
	sprite = Sprite2D.new()
	sprite.position = Vector2(64, 48)
	sprite.set_meta("qa_id", "sprite")
	var image := Image.create(32, 32, false, Image.FORMAT_RGBA8)
	image.fill(Color.TRANSPARENT)
	# Opaque border with a transparent center must be measured across actual
	# Sprite2D geometry; the old origin-only 4x4 sample sees zero contribution.
	for x in 32:
		for y in 32:
			if x < 4 or x >= 28 or y < 4 or y >= 28:
				image.set_pixel(x, y, Color(1, 0, 0, 1))
	sprite.texture = ImageTexture.create_from_image(image)
	scene.add_child(sprite)
	cover = AnimatedCover.new()
	cover.position = Vector2(40, 24)
	cover.size = Vector2(48, 48)
	cover.color = Color.BLUE
	cover.z_index = 10
	cover.visible = false
	scene.add_child(cover)
	player = AudioStreamPlayer.new()
	player.set_meta("qa_id", "player")
	scene.add_child(player)
	runtime = Runtime.new(scene, get_root(), [], {})
	var facts := await snapshot("visible")
	check("transparent-center sprite contributes visible pixels", facts.visible_pixel_count > 0, facts)
	check("capture facts were observed on rendered frames", facts.rendered_frames > 0 and not facts.binding_changed, facts)
	sprite.visible = false
	facts = await snapshot("hidden")
	check("hidden sprite is unconfirmed", facts.visible_pixel_count == 0, facts)
	sprite.visible = true
	sprite.position = Vector2(-5000, -5000)
	facts = await snapshot("offscreen")
	check("offscreen sprite is unconfirmed", facts.visible_pixel_count == 0, facts)
	sprite.position = Vector2(64, 48)
	cover.visible = true
	facts = await snapshot("occluded")
	check("occluded sprite is unconfirmed", facts.visible_pixel_count == 0, facts)
	cover.animate = true
	facts = await snapshot("animated_occluder")
	check("animated occluder cannot supply texture pixels", facts.visible_pixel_count == 0, facts)
	cover.animate = false
	cover.visible = false
	var saved_texture := sprite.texture
	var transparent := Image.create(32, 32, false, Image.FORMAT_RGBA8)
	transparent.fill(Color.TRANSPARENT)
	sprite.texture = ImageTexture.create_from_image(transparent)
	var visible_child := ColorRect.new()
	visible_child.position = Vector2(-8, -8)
	visible_child.size = Vector2(16, 16)
	visible_child.color = Color.GREEN
	sprite.add_child(visible_child)
	facts = await snapshot("child_only")
	check("visible child cannot supply transparent texture pixels", facts.visible_pixel_count == 0, facts)
	visible_child.free()
	sprite.texture = saved_texture
	facts = await snapshot("rebound", true)
	check("rendered binding rebind is rejected", facts.binding_changed, facts)
	var duplicate := Sprite2D.new()
	duplicate.set_meta("qa_id", "sprite")
	scene.add_child(duplicate)
	facts = await snapshot("ambiguous")
	check("ambiguous runtime target is unconfirmed", facts.has("error") and facts.visible_pixel_count == 0, facts)
	duplicate.free()
	var stream := AudioStreamWAV.new()
	stream.format = AudioStreamWAV.FORMAT_16_BITS
	stream.mix_rate = 44100
	var bytes := PackedByteArray()
	bytes.resize(44100 * 2 * 3)
	for index in 44100 * 3:
		bytes.encode_s16(index * 2, int(sin(float(index) * TAU * 440.0 / 44100.0) * 16000))
	stream.data = bytes
	player.stream = stream
	var collector := audio_start("idle")
	facts = audio_facts(collector, "idle")
	check("idle audio has no playback overlap", not facts.playback_overlap, facts)
	player.play()
	await create_timer(0.04).timeout
	player.stream_paused = true
	collector = audio_start("paused")
	facts = audio_facts(collector, "paused")
	check("paused audio has no playback overlap", facts.stream_paused and not facts.playback_overlap, facts)
	player.stream_paused = false
	collector = audio_start("playing")
	await process_frame
	facts = audio_facts(collector, "playing")
	check("playing audio has observed overlap", facts.playback_overlap and facts.bus_matches and not facts.binding_changed, facts)
	AudioServer.add_bus()
	AudioServer.set_bus_name(AudioServer.bus_count - 1, "Other")
	collector = audio_start("wrong_bus", "Other")
	facts = audio_facts(collector, "wrong_bus")
	check("audio on another bus is unconfirmed", not facts.playback_overlap and not facts.bus_matches, facts)
	collector = audio_start("audio_rebound")
	player.stream = stream.duplicate()
	collector.audio_frame(runtime)
	player.stream = stream
	facts = audio_facts(collector, "audio_rebound")
	check("audio rebind-and-restore is rejected", facts.binding_changed, facts)
	collector = audio_start("audio_mutated")
	bytes.encode_s16(0, 1234)
	stream.data = bytes
	facts = audio_facts(collector, "audio_mutated")
	check("in-place stream data mutation is rejected", facts.binding_changed, facts)
	runtime.teardown()
	scene.free()
	runtime = null
	var report := FileAccess.open("res://report.json", FileAccess.WRITE)
	report.store_string(JSON.stringify({"checks": checks, "failures": failures}))
	report.close()
	call_deferred("quit", 0 if failures.is_empty() else 1)
