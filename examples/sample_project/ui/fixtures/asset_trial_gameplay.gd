extends Node2D

var _pickup_count := 0
var _scheduled := false

@onready var coin: Node2D = $CoinPickup
@onready var pickup_timer: Timer = $PickupTimer
@onready var pickup_signal: Node = $PickupSignal


func _draw() -> void:
	for row in 15:
		for column in 20:
			var hue := fmod(float(column) * 0.17 + float(row) * 0.071, 1.0)
			var value := 0.28 + float((column + row) % 4) * 0.055
			draw_rect(Rect2(column * 48, row * 36, 48, 36), Color.from_hsv(hue, 0.52, value))


func _ready() -> void:
	set_meta("qa_id", "asset_trial_gameplay")
	add_to_group("qa.id.asset_trial_gameplay")
	var bus_index := AudioServer.get_bus_index("SFX")
	if bus_index < 0:
		AudioServer.add_bus()
		bus_index = AudioServer.get_bus_count() - 1
		AudioServer.set_bus_name(bus_index, "SFX")
	AudioServer.set_bus_send(bus_index, "Master")
	coin.get_node("PickupSound").bus = "SFX"


func qa_schedule_pickup(delay_seconds: float = 0.3) -> Dictionary:
	if _scheduled:
		return {"ok": false, "reason": "already_scheduled"}
	_scheduled = true
	pickup_timer.wait_time = delay_seconds
	pickup_timer.start()
	return {"ok": true, "delay_seconds": delay_seconds}


func get_asset_trial_qa_state() -> Dictionary:
	return {"pickup_count": _pickup_count, "scheduled": _scheduled}


func _on_pickup_timer_timeout() -> void:
	pickup_signal.emit_signal("picked_up")
	coin.get_node("PickupSound").play()
	coin.get_node("CoinSprite").queue_free()


func _on_coin_picked_up() -> void:
	_pickup_count += 1
