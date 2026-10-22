extends SceneTree

const FIXTURE_DIR := "res://ui/fixtures"


func _initialize() -> void:
	var decoration_scene_root := Node2D.new()
	decoration_scene_root.name = "Decoration"
	var decoration_marker := Polygon2D.new()
	decoration_marker.name = "Marker"
	decoration_marker.polygon = PackedVector2Array([Vector2(-5, -5), Vector2(5, -5), Vector2(0, 5)])
	decoration_marker.color = Color(0.2, 0.7, 0.95, 1.0)
	decoration_scene_root.add_child(decoration_marker)
	decoration_marker.owner = decoration_scene_root
	var decoration_scene := PackedScene.new()
	assert(decoration_scene.pack(decoration_scene_root) == OK)
	assert(ResourceSaver.save(decoration_scene, FIXTURE_DIR + "/asset_coin_decoration.tscn") == OK)
	decoration_scene_root.free()

	var coin := Node2D.new()
	coin.name = "AssetCoin"
	coin.set_script(load("res://ui/fixtures/asset_coin.gd"))
	var sprite := Sprite2D.new()
	sprite.name = "CoinSprite"
	sprite.texture = load("res://assets/asset_trial/coin_original.png")
	sprite.position = Vector2(0, -8)
	sprite.set_meta("qa_id", "asset_coin_sprite")
	sprite.add_to_group("qa.id.asset_coin_sprite")
	coin.add_child(sprite)
	sprite.owner = coin
	var pickup_sound := AudioStreamPlayer.new()
	pickup_sound.name = "PickupSound"
	pickup_sound.bus = "SFX"
	pickup_sound.set_meta("qa_id", "asset_coin_sfx")
	pickup_sound.add_to_group("qa.id.asset_coin_sfx")
	coin.add_child(pickup_sound)
	pickup_sound.owner = coin
	pickup_sound.finished.connect(Callable(coin, "qa_on_sound_finished"), Object.CONNECT_PERSIST)
	var decoration: Node2D = load(FIXTURE_DIR + "/asset_coin_decoration.tscn").instantiate()
	decoration.name = "UnrelatedDecoration"
	decoration.position = Vector2(36, 0)
	decoration.modulate = Color(0.4, 0.8, 1.0, 1.0)
	coin.add_child(decoration)
	decoration.owner = coin
	var coin_scene := PackedScene.new()
	assert(coin_scene.pack(coin) == OK)
	assert(ResourceSaver.save(coin_scene, FIXTURE_DIR + "/asset_coin.tscn") == OK)
	coin.free()

	var gameplay := Node2D.new()
	gameplay.name = "AssetTrialGameplay"
	gameplay.set_script(load("res://ui/fixtures/asset_trial_gameplay.gd"))
	var coin_instance: Node2D = load(FIXTURE_DIR + "/asset_coin.tscn").instantiate()
	coin_instance.name = "CoinPickup"
	coin_instance.position = Vector2(430, 250)
	gameplay.add_child(coin_instance)
	coin_instance.owner = gameplay
	var extra := Node2D.new()
	extra.name = "UnrelatedInstance"
	extra.position = Vector2(680, 240)
	extra.modulate = Color(0.75, 0.5, 1.0, 1.0)
	var extra_marker := Polygon2D.new()
	extra_marker.name = "Marker"
	extra_marker.polygon = PackedVector2Array([Vector2(-28, -28), Vector2(28, -28), Vector2(28, 28), Vector2(-28, 28)])
	extra_marker.color = Color(0.2, 0.4, 0.8, 1.0)
	extra.add_child(extra_marker)
	gameplay.add_child(extra)
	extra.owner = gameplay
	extra_marker.owner = gameplay
	var timer := Timer.new()
	timer.name = "PickupTimer"
	timer.one_shot = true
	gameplay.add_child(timer)
	timer.owner = gameplay
	timer.timeout.connect(Callable(gameplay, "_on_pickup_timer_timeout"), Object.CONNECT_PERSIST)
	var signal_source := Node.new()
	signal_source.name = "PickupSignal"
	signal_source.set_script(load("res://ui/fixtures/asset_trial_pickup_signal.gd"))
	gameplay.add_child(signal_source)
	signal_source.owner = gameplay
	signal_source.connect("picked_up", Callable(gameplay, "_on_coin_picked_up"), Object.CONNECT_PERSIST)
	var gameplay_scene := PackedScene.new()
	assert(gameplay_scene.pack(gameplay) == OK)
	assert(ResourceSaver.save(gameplay_scene, FIXTURE_DIR + "/asset_trial_gameplay.tscn") == OK)
	gameplay.free()
	quit()
