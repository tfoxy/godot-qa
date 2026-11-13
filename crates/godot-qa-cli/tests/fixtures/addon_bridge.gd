extends SceneTree

const Runtime = preload("res://addons/godot_qa/qa_runtime.gd")
const Provider = preload("res://provider.gd")

const Host = preload("res://addons/godot_qa/qa_live_session_host.gd")

class CountingBridge extends Node:
	var calls := 0
	var errors: Array = []

	func handle_command(_request: Dictionary) -> Dictionary:
		calls += 1
		return {"ok": true, "data": {"executions": calls}}

	func record_runtime_error(_message: String, _source: String, extra: Dictionary) -> void:
		errors.append(extra)

const Bridge = preload("res://addons/godot_qa/qa_bridge.gd")

var checks: Array = []
var failures: Array = []
var bridge: Node


func _initialize() -> void:
	call_deferred("run")


func check(label: String, condition: bool, actual: Variant) -> void:
	checks.append(label)
	if not condition:
		failures.append({"check": label, "actual": str(actual)})


func command(name: String, params: Dictionary = {}) -> Dictionary:
	return await bridge.handle_command({"command": name, "params": params, "token": "test-token"})


func temporal_paths() -> void:
	var subject := Node2D.new()
	subject.set_meta("qa_id", "subject")
	get_root().add_child(subject)
	current_scene = subject
	var commands := {
		"assert.temporal.trend": {"target": "qa:subject", "property": "position", "trend": "increasing", "min_delta": 1, "window_frames": 2},
		"assert.temporal.frame_time": {"window_frames": 2, "max_avg_ms": 0.000001},
	}
	for name in commands:
		var params: Dictionary = commands[name].duplicate(true)
		params["temporal_dir"] = "res://outside"
		var rejected := await command(name, params)
		check("outside directory rejected", rejected.get("error", {}).get("type") == "invalid_params", rejected)
		check("outside directory untouched", not DirAccess.dir_exists_absolute("res://outside"), rejected)
		for session in ["one", "two"]:
			bridge._live_session["artifacts_root"] = "res://sessions/" + session
			var result := await command(name, commands[name])
			var evidence := str(result.get("error", {}).get("data", {}).get("artifacts", {}).get("temporal_evidence", ""))
			check("evidence isolated in " + session, evidence.begins_with("res://sessions/" + session + "/temporal/") and FileAccess.file_exists(evidence), result)
		bridge._live_session.erase("artifacts_root")
		var missing := await command(name, commands[name])
		check("missing root rejected", missing.get("error", {}).get("type") == "artifact_root_missing", missing)
		bridge._live_session["artifacts_root"] = "res://sessions/one"
	subject.free()


func save_scene(provider: bool) -> void:
	var node := Node.new()
	node.name = "Subject"
	if provider:
		node.set_script(Provider)
	var packed := PackedScene.new()
	check("scene packed", packed.pack(node) == OK, "pack")
	check("scene saved", ResourceSaver.save(packed, "res://subject.tscn") == OK, "save")
	node.free()
	var scene: Node = load("res://subject.tscn").instantiate()
	get_root().add_child(scene)
	current_scene = scene
	bridge._live_session["scene"] = "res://subject.tscn"


func invalid_scene_change() -> void:
	save_scene(false)
	for path in ["res://missing.tscn", "res://provider.gd"]:
		var original := current_scene
		var changed := await command("session.change_scene", {"scene": path})
		check("invalid change failed", not changed.get("ok", true), changed)
		check("original scene kept", current_scene == original, changed)
		var status := await command("session.status")
		check("status kept original path", status.get("data", {}).get("scene") == "res://subject.tscn", status)
		var reloaded := await command("session.reload_scene")
		check("reload uses original path", reloaded.get("ok", false) and reloaded.get("data", {}).get("scene") == "res://subject.tscn", reloaded)
	current_scene.free()


func scene_providers() -> void:
	var autoload_provider := Provider.new()
	# An unrelated provider outside the scene subtree must survive every swap.
	get_root().add_child(autoload_provider)
	Runtime.unregister_state_provider("scene-provider")
	Runtime.unregister_state_provider("child-provider")
	Runtime.register_state_provider("autoload-provider", autoload_provider, "qa_state")
	save_scene(true)
	var runtime := Runtime.new(current_scene, get_root())
	for index in range(3):
		var old_id := current_scene.get_instance_id()
		var result := await command("session.reload_scene")
		check("reload succeeded", result.get("ok", false), result)
		check("ready registered replacement", current_scene.registration.get("ok", false), current_scene.registration)
		var state: Dictionary = runtime.read_state_provider("scene-provider")
		check("provider points to replacement", state.get("ok", false) and state.get("state", {}).get("instance") == current_scene.get_instance_id() and old_id != current_scene.get_instance_id(), state)
		var child_state: Dictionary = runtime.read_state_provider("child-provider")
		check("child provider released without exit hook", child_state.get("ok", false) and child_state.get("state", {}).get("instance") == current_scene.get_node("ChildProvider").get_instance_id(), child_state)
		var autoload_state: Dictionary = runtime.read_state_provider("autoload-provider")
		check("autoload provider survived", autoload_state.get("ok", false), autoload_state)
	runtime.teardown()
	current_scene.free()
	Runtime.unregister_state_provider("autoload-provider")
	autoload_provider.free()


func scene_providers_stale_owner(unrelated: bool) -> void:
	if unrelated:
		# Register before scene entries so a stale owner cannot skip their cleanup.
		var stale := Provider.ChildProvider.new()
		var registered := Runtime.register_state_provider("aaa-stale", stale, "qa_state")
		check("unrelated provider registered", registered.get("ok", false), registered)
		stale.free()
	save_scene(true)
	if not unrelated:
		current_scene.get_node("ChildProvider").free()
	# Do not look up providers before reload: lookups prune invalid owners.
	var result := await command("session.reload_scene")
	check("reload with stale owner succeeded", result.get("ok", false), result)
	check("replacement registered", current_scene.registration.get("ok", false), current_scene.registration)
	var runtime := Runtime.new(current_scene, get_root())
	var state: Dictionary = runtime.read_state_provider("child-provider")
	check("child provider resolves to replacement", state.get("ok", false) and state.get("state", {}).get("instance") == current_scene.get_node("ChildProvider").get_instance_id(), state)
	runtime.teardown()
	current_scene.free()
	Runtime.clear_provider_registry()


func settled_object_count() -> int:
	await process_frame
	await process_frame
	return int(Performance.get_monitor(Performance.OBJECT_COUNT))


func runtime_cleanup() -> void:
	save_scene(false)
	await command("session.status")
	await command("unsupported.test.command")
	var before := await settled_object_count()
	for index in range(250):
		await command("session.status")
		await command("session.change_scene")
		await command("unsupported.test.command")
	var after := await settled_object_count()
	check("status and early/default returns release runtimes", after <= before + 4, {"before": before, "after": after})
	before = after
	for index in range(20):
		await command("session.reload_scene")
	after = await settled_object_count()
	check("scene input cleanup runtime released", after <= before + 4, {"before": before, "after": after})
	bridge._live_session["recorder_enabled"] = true
	bridge._reinitialize_recorder()
	before = await settled_object_count()
	for index in range(100):
		bridge._reinitialize_recorder()
	after = await settled_object_count()
	check("recorder replacement releases runtime", after <= before + 4, {"before": before, "after": after})
	bridge._live_session["recorder_enabled"] = false
	bridge._reinitialize_recorder()
	await command("session.profile.soak_start", {"run_id": "invalid", "spec": {}})
	before = await settled_object_count()
	for index in range(100):
		await command("session.profile.soak_start", {"run_id": "invalid", "spec": {}})
	after = await settled_object_count()
	check("failed soak startup releases runtime", after <= before + 4, {"before": before, "after": after})
	var started := await command("session.profile.soak_start", {"run_id": "stopped", "spec": {"version": 1, "duration_seconds": 1, "window_seconds": 1}})
	check("soak started", started.get("ok", false), started)
	await command("session.profile.soak_stop", {"run_id": "stopped"})
	check("stopped soak runtime released", bridge._soak != null and bridge._soak.runtime == null, "soak runtime")
	current_scene.free()


func file_publication() -> void:
	var counter := CountingBridge.new()
	var host := Host.new()
	get_root().add_child(counter)
	get_root().add_child(host)
	host._bridge = counter
	host._file_requests_dir = "res://rpc/requests"
	host._file_responses_dir = "res://blocked"
	DirAccess.make_dir_recursive_absolute(host._file_requests_dir)
	var blocker := FileAccess.open("res://blocked", FileAccess.WRITE)
	blocker.store_string("not a directory")
	blocker.close()
	var request := "res://rpc/requests/once.json"
	var file := FileAccess.open(request, FileAccess.WRITE)
	file.store_string(JSON.stringify({"command": "mutation", "token": "test-token"}))
	file.close()
	for index in range(3):
		host._handle_file_transport()
		await process_frame
	check("mutation executed once", counter.calls == 1, counter.calls)
	check("unpublished request retained", FileAccess.file_exists(request), request)
	check("publication failure surfaced once", counter.errors.size() == 1 and counter.errors[0].get("code") == "file_transport_publish_failed", counter.errors)
	check("completed response retained", host._processed_file_requests.get("once.json") is Dictionary, host._processed_file_requests)
	DirAccess.remove_absolute(request) # The client can time out and remove its request.
	host._prune_processed_file_requests()
	check("completed result survives client timeout", host._processed_file_requests.get("once.json") is Dictionary, host._processed_file_requests)
	DirAccess.remove_absolute("res://blocked")
	DirAccess.make_dir_recursive_absolute("res://blocked")
	host._handle_file_transport()
	await process_frame
	var response: Variant = JSON.parse_string(FileAccess.get_file_as_string("res://blocked/once.json"))
	check("original result recovered", response is Dictionary and response.get("data", {}).get("executions") == 1, response)
	check("request removed only after publication", not FileAccess.file_exists(request) and counter.calls == 1, counter.calls)
	host._prune_processed_file_requests()
	check("published cache pruned", host._processed_file_requests.is_empty(), host._processed_file_requests)
	host.free()
	counter.free()


func connect_client(port: int) -> StreamPeerTCP:
	var peer := StreamPeerTCP.new()
	peer.connect_to_host("127.0.0.1", port)
	for index in range(30):
		peer.poll()
		if peer.get_status() != StreamPeerTCP.STATUS_CONNECTING:
			break
		await process_frame
	return peer


func client_response(peer: StreamPeerTCP) -> Dictionary:
	var text := ""
	for index in range(120):
		peer.poll()
		if peer.get_status() != StreamPeerTCP.STATUS_CONNECTED:
			break
		if peer.get_available_bytes() > 0:
			text += peer.get_utf8_string(peer.get_available_bytes())
			if text.contains("\n"):
				var response: Variant = JSON.parse_string(text.get_slice("\n", 0))
				return response if response is Dictionary else {}
		await process_frame
	return {}


func transport_limits() -> void:
	var host := Host.new()
	get_root().add_child(host)
	var configured: Dictionary = host.configure(bridge, 0)
	check("host configured", configured.get("ok", false), configured)
	var port: int = host._server.get_local_port()
	host.serve()
	var clients: Array[StreamPeerTCP] = []
	for index in range(40):
		clients.append(await connect_client(port))
		await process_frame
	check("pre-auth connections capped", host._in_flight <= 32, host._in_flight)
	for peer in clients:
		peer.disconnect_from_host()
	for index in range(4):
		await process_frame
	var oversized := await connect_client(port)
	var chunk := "x".repeat(65536).to_utf8_buffer()
	for index in range(17):
		oversized.poll()
		if oversized.get_status() != StreamPeerTCP.STATUS_CONNECTED:
			break
		oversized.put_data(chunk)
		await process_frame
	var rejected := await client_response(oversized)
	check("oversized unterminated request rejected", rejected.get("error", {}).get("type") == "protocol_error" or oversized.get_status() != StreamPeerTCP.STATUS_CONNECTED, rejected)
	check("oversized request released promptly", host._in_flight == 0, host._in_flight)
	var valid := await connect_client(port)
	valid.put_data((JSON.stringify({"command": "session.status", "token": "test-token"}) + "\n").to_utf8_buffer())
	var response := await client_response(valid)
	check("valid client still served", response.get("ok", false), response)
	oversized.disconnect_from_host()
	valid.disconnect_from_host()
	bridge._stop_requested = true
	await process_frame
	await process_frame
	host.free()


func log_retention() -> void:
	var initial := int(bridge._read_logs("").next_cursor)
	for index in range(5000):
		bridge._record_log("info", "retention_test", "entry", {"index": index})
	var response := await command("logs.read")
	var data: Dictionary = response.get("data", {})
	var entries: Array = data.get("entries", [])
	var last_cursor := initial + 5000
	var oldest := last_cursor - 2048 + 1
	check("retention bounded", bridge._session_logs.size() == 2048 and entries.size() == 2048, entries.size())
	check("cursor independent of retention", data.get("next_cursor") == str(last_cursor), data.get("next_cursor"))
	check("oldest retained cursor reported", data.get("oldest_cursor") == str(oldest), data.get("oldest_cursor"))
	check("omitted history reported", data.get("truncated", false) and data.get("dropped_entries") == oldest - 1, data.get("dropped_entries"))
	if not entries.is_empty():
		check("retained cursor boundaries", entries[0].cursor == str(oldest) and entries[-1].cursor == str(last_cursor), [entries[0].cursor, entries[-1].cursor])
	for since in [oldest - 2, oldest - 1, last_cursor - 10, last_cursor, last_cursor + 10]:
		var read := await command("logs.read", {"since": str(since)})
		var result: Dictionary = read.get("data", {})
		var expected_count := mini(2048, maxi(last_cursor - since, 0))
		check("cursor read size " + str(since), result.get("entries", []).size() == expected_count, result.get("entries", []).size())
		check("truncation at cursor " + str(since), result.get("truncated") == (since < oldest - 1) and result.get("dropped_entries") == maxi(oldest - 1 - since, 0), result.get("dropped_entries"))
	bridge._record_log("info", "retention_test", "next entry")
	var next := await command("logs.read", {"since": str(last_cursor)})
	var tail: Dictionary = next.get("data", {})
	check("new entries remain incrementally readable", tail.get("entries", []).size() == 1 and tail.get("next_cursor") == str(last_cursor + 1), tail)
	check("retention remains bounded", bridge._session_logs.size() == 2048, bridge._session_logs.size())


func run() -> void:
	bridge = Bridge.new()
	get_root().add_child(bridge)
	bridge.enabled = true
	bridge._android_session_token = "test-token"
	bridge.configure_live_session({"session_id": "test", "artifacts_root": "res://sessions/one"})
	match OS.get_cmdline_user_args()[0]:
		"temporal_paths":
			await temporal_paths()
		"invalid_scene_change":
			await invalid_scene_change()
		"scene_providers":
			await scene_providers()
		"scene_providers_freed_child":
			await scene_providers_stale_owner(false)
		"scene_providers_unrelated_stale_owner":
			await scene_providers_stale_owner(true)
		"runtime_cleanup":
			await runtime_cleanup()
		"file_publication":
			await file_publication()
		"transport_limits":
			await transport_limits()
		"log_retention":
			await log_retention()
	bridge.free()
	var file := FileAccess.open("res://report.json", FileAccess.WRITE)
	file.store_string(JSON.stringify({"checks": checks, "failures": failures}))
	file.close()
	quit(0 if failures.is_empty() else 1)
