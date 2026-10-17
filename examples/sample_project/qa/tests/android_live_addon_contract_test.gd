extends SceneTree

const QaBridge = preload("res://addons/godot_qa/qa_bridge.gd")


func _initialize() -> void:
	var bridge := QaBridge.new()
	var failures: Array[String] = []
	_expect(bridge._is_safe_android_session_id("session_123-A"), "safe session id rejected", failures)
	_expect(not bridge._is_safe_android_session_id("../escape"), "traversal session id accepted", failures)
	_expect(bridge._is_safe_android_scene_path("res://ui/main_menu.tscn"), "safe scene rejected", failures)
	_expect(bridge._is_safe_android_scene_path("res://ui/main_menu.scn"), "safe binary scene rejected", failures)
	_expect(bridge._is_safe_android_scene_path(""), "packaged-scene bootstrap rejected", failures)
	_expect(not bridge._is_safe_android_scene_path("res://../outside.tscn"), "traversal scene accepted", failures)
	_expect(bridge._sanitize_capture_name("../../outside") == "outside", "capture name was not sanitized", failures)
	bridge._android_session_token = "private-test-token"
	_expect(bridge._configured_session_token() == "private-test-token", "private Android token was not used", failures)
	_expect(
		not JSON.stringify(bridge.get_status()).contains("private-test-token"),
		"private Android token leaked through bridge status",
		failures,
	)
	bridge._android_session_token = ""
	var inactive_response: Dictionary = await bridge.handle_command({
		"command": "session.status",
		"token": "inactive-secret-token",
	})
	_expect(
		not JSON.stringify(inactive_response).contains("inactive-secret-token"),
		"inactive bridge response leaked the supplied token",
		failures,
	)

	var bootstrap_dir := ProjectSettings.globalize_path("user://godot_qa")
	DirAccess.make_dir_recursive_absolute(bootstrap_dir)
	var bootstrap_path := ProjectSettings.globalize_path(bridge.ANDROID_BOOTSTRAP_PATH)
	var bootstrap_file := FileAccess.open(bootstrap_path, FileAccess.WRITE)
	if bootstrap_file == null:
		failures.append("bootstrap fixture could not be written")
	else:
		bootstrap_file.store_string(JSON.stringify({
			"version": 1,
			"session": {
				"id": "contract-test",
				"token": "private-test-token",
				"port": 32001,
				"scene": "res://ui/main_menu.tscn",
				"started_at": "test",
				"project_args": ["--fixture=android"],
				"recorder_enabled": false,
				"recorder": {},
			},
		}))
		bootstrap_file.close()
		var parsed: Dictionary = bridge._read_android_bootstrap()
		_expect(bool(parsed.get("ok", false)), "valid bootstrap was rejected", failures)
		var parsed_session: Dictionary = parsed.get("session", {})
		_expect(str(parsed_session.get("id", "")) == "contract-test", "bootstrap session id changed", failures)

		var invalid_file := FileAccess.open(bootstrap_path, FileAccess.WRITE)
		if invalid_file == null:
			failures.append("invalid bootstrap fixture could not be written")
		else:
			invalid_file.store_string(JSON.stringify({
				"version": 1,
				"session": {
					"id": "../escape",
					"token": "private-test-token",
					"port": 32001,
				},
			}))
			invalid_file.close()
			var invalid: Dictionary = bridge._read_android_bootstrap()
			_expect(not bool(invalid.get("ok", false)), "unsafe bootstrap was accepted", failures)

		# Optional bootstrap `mode` key: absent defaults to live; live/scenario are
		# valid; an unknown mode string or any unknown extra key is rejected.
		var base_session := {
			"id": "contract-test",
			"token": "private-test-token",
			"port": 32001,
			"scene": "res://ui/main_menu.tscn",
			"started_at": "test",
			"project_args": [],
			"recorder_enabled": false,
			"recorder": {},
		}
		var absent: Dictionary = _read_session(bridge, bootstrap_path, base_session)
		_expect(bool(absent.get("ok", false)), "absent mode bootstrap was rejected", failures)
		_expect(
			str((absent.get("session", {}) as Dictionary).get("mode", "")) == "live",
			"absent mode did not default to live",
			failures,
		)

		var live_session := base_session.duplicate(true)
		live_session["mode"] = "live"
		_expect(
			bool(_read_session(bridge, bootstrap_path, live_session).get("ok", false)),
			"live mode bootstrap was rejected",
			failures,
		)

		var scenario_session := base_session.duplicate(true)
		scenario_session["mode"] = "scenario"
		var scenario: Dictionary = _read_session(bridge, bootstrap_path, scenario_session)
		_expect(bool(scenario.get("ok", false)), "scenario mode bootstrap was rejected", failures)
		_expect(
			str((scenario.get("session", {}) as Dictionary).get("mode", "")) == "scenario",
			"scenario mode was not stored on the session",
			failures,
		)

		var bad_mode := base_session.duplicate(true)
		bad_mode["mode"] = "ludicrous"
		_expect(
			not bool(_read_session(bridge, bootstrap_path, bad_mode).get("ok", false)),
			"unknown mode string was accepted",
			failures,
		)

		var extra_key := base_session.duplicate(true)
		extra_key["surprise"] = true
		_expect(
			not bool(_read_session(bridge, bootstrap_path, extra_key).get("ok", false)),
			"unknown extra session key was accepted",
			failures,
		)
	DirAccess.remove_absolute(bootstrap_path)
	bridge.free()

	if failures.is_empty():
		print("android live addon contract: ok")
		quit(0)
		return
	for failure in failures:
		push_error(failure)
	quit(1)


func _read_session(bridge, bootstrap_path: String, session: Dictionary) -> Dictionary:
	var file := FileAccess.open(bootstrap_path, FileAccess.WRITE)
	if file == null:
		return {"ok": false, "reason": "fixture_write_failed"}
	file.store_string(JSON.stringify({"version": 1, "session": session}))
	file.close()
	return bridge._read_android_bootstrap()


func _expect(condition: bool, message: String, failures: Array[String]) -> void:
	if not condition:
		failures.append(message)
