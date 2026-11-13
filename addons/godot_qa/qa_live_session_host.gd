extends Node

const QaProtocol = preload("res://addons/godot_qa/qa_protocol.gd")

## Milliseconds the serve loop sleeps per frame while no command is in flight.
## Deliberately host-local: `Engine.max_fps` and `OS.low_processor_usage_mode`
## are batch-global engine state the scenario executor snapshots and guards
## against contamination, so this host must never touch them.
const IDLE_FRAME_DELAY_MSEC := 1
const MAX_REQUEST_BYTES := 1024 * 1024
const MAX_TCP_CONNECTIONS := 32

## Poll the file transport every Nth idle frame instead of every frame. The
## directory listing is the expensive part of an otherwise empty idle frame.
const IDLE_FILE_TRANSPORT_POLL_FRAMES := 8

var _bridge: Node = null
var _server := TCPServer.new()
var _tcp_enabled := false
var _allow_file_transport := false
var _file_requests_dir := ""
var _file_responses_dir := ""
var _processed_file_requests := {}
var _in_flight := 0
var _tcp_connections := 0
var _idle_frames := 0


func configure(bridge: Node, port: int, artifact_root: String = "", allow_file_transport: bool = false) -> Dictionary:
	_bridge = bridge
	_allow_file_transport = allow_file_transport
	if _allow_file_transport:
		_configure_file_transport(artifact_root)

	var listen_error := _server.listen(port, "127.0.0.1")
	if listen_error != OK:
		return QaProtocol.error(
			"session_listen_failed",
			"live session TCP server could not bind requested port",
			{
				"port": port,
				"actual": listen_error,
				"hint": "another process may hold the port; retry `session start`",
				"file_transport": _file_requests_dir != "",
			},
		)
	_tcp_enabled = true
	return QaProtocol.ok({"port": port, "bind": "127.0.0.1"})


func serve() -> void:
	while true:
		if _bridge != null and _bridge.has_method("consume_stop_requested") and bool(_bridge.call("consume_stop_requested")):
			break
		if _tcp_enabled and _server.is_connection_available():
			var peer := _server.take_connection()
			_track_connection(peer)
			continue
		if _in_flight > 0:
			# A command is in flight. The live-session request timeout and
			# `session.wait_frames` frame budget are both measured in real
			# frames, so this path stays exactly as fast as it was: poll the
			# file transport every frame and never sleep.
			_idle_frames = 0
			if _allow_file_transport:
				_handle_file_transport()
			await get_tree().process_frame
			continue
		if _allow_file_transport and _idle_frames % IDLE_FILE_TRANSPORT_POLL_FRAMES == 0:
			_prune_processed_file_requests()
			_handle_file_transport()
		_idle_frames += 1
		# `_handle_file_transport` may have just launched a request; re-check so
		# a freshly picked up command never pays the idle sleep.
		if _in_flight == 0:
			OS.delay_msec(IDLE_FRAME_DELAY_MSEC)
		await get_tree().process_frame
	_server.stop()


## Runs `_handle_connection` while the host counts it as in flight. A leaked
## increment can only disable the idle throttle (today's behavior), never
## throttle a live command, so this fails in the safe direction.
func _track_connection(peer: StreamPeerTCP) -> void:
	if _tcp_connections >= MAX_TCP_CONNECTIONS:
		peer.put_data((JSON.stringify(QaProtocol.error("protocol_error", "live session connection limit reached", {"expected": {"max_connections": MAX_TCP_CONNECTIONS}, "actual": {"connections": _tcp_connections + 1}})) + "\n").to_utf8_buffer())
		peer.disconnect_from_host()
		return
	_tcp_connections += 1
	_in_flight += 1
	await _handle_connection(peer)
	_in_flight = maxi(_in_flight - 1, 0)
	_tcp_connections -= 1


## Runs `_handle_file_request` while the host counts it as in flight.
func _track_file_request(file_name: String) -> void:
	_in_flight += 1
	await _handle_file_request(file_name)
	_in_flight = maxi(_in_flight - 1, 0)


func _handle_connection(peer: StreamPeerTCP) -> void:
	var read_result := await _read_request_line(peer)
	var request_text := str(read_result.get("text", ""))
	var response := {}
	if not bool(read_result.get("ok", false)):
		response = read_result.get("response", {})
	elif request_text == "":
		response = QaProtocol.error("protocol_error", "live session request was empty", {})
	else:
		var parser := JSON.new()
		var parse_error := parser.parse(request_text)
		if parse_error != OK or not parser.data is Dictionary:
			response = QaProtocol.error(
				"protocol_error",
				"live session request must be a JSON object",
				{"actual": "invalid_json_or_non_object"},
			)
		else:
			response = await _bridge.handle_command(parser.data)

	var response_text := "%s\n" % JSON.stringify(response)
	peer.put_data(response_text.to_utf8_buffer())
	peer.disconnect_from_host()


func _configure_file_transport(artifact_root: String) -> void:
	if artifact_root == "":
		return
	_file_requests_dir = "%s/rpc/requests" % artifact_root
	_file_responses_dir = "%s/rpc/responses" % artifact_root
	var request_error := DirAccess.make_dir_recursive_absolute(_file_requests_dir)
	var response_error := DirAccess.make_dir_recursive_absolute(_file_responses_dir)
	if request_error != OK or response_error != OK:
		_file_requests_dir = ""
		_file_responses_dir = ""


func _handle_file_transport() -> void:
	if _file_requests_dir == "" or _bridge == null:
		return
	# A completed command is retried only at publication, even if the client
	# has removed its request after timing out.
	for file_name in _processed_file_requests.keys():
		if _processed_file_requests[file_name] is Dictionary:
			_publish_completed_file_request(file_name, _processed_file_requests[file_name])
	var dir := DirAccess.open(_file_requests_dir)
	if dir == null:
		return
	dir.list_dir_begin()
	while true:
		var file_name := dir.get_next()
		if file_name == "":
			break
		if dir.current_is_dir() or not file_name.ends_with(".json"):
			continue
		if _processed_file_requests.has(file_name):
			continue
		_processed_file_requests[file_name] = true
		_track_file_request(file_name)
	dir.list_dir_end()


## Drops processed-request keys whose request file is gone, so a long-lived
## pooled session does not grow this dictionary once per file request forever.
## Only called while nothing is in flight, so no entry being pruned can belong
## to a request that has not yet deleted its own file.
func _prune_processed_file_requests() -> void:
	if _file_requests_dir == "" or _processed_file_requests.is_empty():
		return
	var stale: Array[String] = []
	for file_name in _processed_file_requests:
		if not _processed_file_requests[file_name] is Dictionary and not FileAccess.file_exists("%s/%s" % [_file_requests_dir, file_name]):
			stale.append(file_name)
	for file_name in stale:
		_processed_file_requests.erase(file_name)


func _handle_file_request(file_name: String) -> void:
	var request_path := "%s/%s" % [_file_requests_dir, file_name]
	var request := _read_json_file(request_path)
	var response := {}
	if request.is_empty():
		response = QaProtocol.error("protocol_error", "live session file request was empty", {})
	else:
		response = await _bridge.handle_command(request)
	var completed := {"response": response, "command": str(request.get("command", "")), "publication_error": {}}
	_processed_file_requests[file_name] = completed
	_publish_completed_file_request(file_name, completed)


func _publish_completed_file_request(file_name: String, completed: Dictionary) -> void:
	var request_path := "%s/%s" % [_file_requests_dir, file_name]
	var response_path := "%s/%s" % [_file_responses_dir, file_name]
	var response_tmp_path := "%s.tmp" % response_path
	var write_error := _write_json_file(response_tmp_path, completed.response)
	if write_error == OK:
		write_error = DirAccess.rename_absolute(response_tmp_path, response_path)
	if write_error != OK:
		write_error = _write_json_file(response_path, completed.response)
	if write_error != OK:
		if (completed.publication_error as Dictionary).is_empty():
			var details := {"code": "file_transport_publish_failed", "command": completed.command, "target": response_path, "request_path": request_path, "response_path": response_path, "expected": "persisted command response before request removal", "actual": {"error": write_error, "message": error_string(write_error)}}
			completed.publication_error = details
			_bridge.record_runtime_error("live file response publication failed; completed result retained for publication retry", "file_transport", details)
		return
	_processed_file_requests[file_name] = true
	DirAccess.remove_absolute(request_path)


func _read_request_line(peer: StreamPeerTCP) -> Dictionary:
	var buffer := PackedByteArray()
	var started_at := Time.get_ticks_msec()
	while Time.get_ticks_msec() - started_at < 5000:
		peer.poll()
		if peer.get_status() != StreamPeerTCP.STATUS_CONNECTED:
			break
		var available := peer.get_available_bytes()
		if available > 0:
			# Read at most the remaining allowance plus one rejection byte. Only
			# scan the new chunk, never the retained prefix of an incomplete line.
			var read := peer.get_data(mini(available, MAX_REQUEST_BYTES - buffer.size() + 1))
			if read[0] != OK:
				break
			var chunk: PackedByteArray = read[1]
			var newline_index := chunk.find(10)
			var line_bytes := newline_index + 1 if newline_index >= 0 else chunk.size()
			if buffer.size() + line_bytes > MAX_REQUEST_BYTES:
				return {"ok": false, "response": QaProtocol.error("protocol_error", "live session request exceeds byte limit", {"expected": {"max_request_bytes": MAX_REQUEST_BYTES}, "actual": {"request_bytes": buffer.size() + line_bytes}})}
			if newline_index >= 0:
				buffer.append_array(chunk.slice(0, newline_index))
				return {"ok": true, "text": buffer.get_string_from_utf8()}
			buffer.append_array(chunk)
		await get_tree().process_frame
	return {"ok": true, "text": buffer.get_string_from_utf8().strip_edges()}


func _read_json_file(path: String) -> Dictionary:
	var file := FileAccess.open(path, FileAccess.READ)
	if file == null:
		return {}
	var parser := JSON.new()
	var parse_error := parser.parse(file.get_as_text())
	if parse_error != OK or not parser.data is Dictionary:
		return {}
	return parser.data


func _write_json_file(path: String, payload: Dictionary) -> Error:
	var file := FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		return FileAccess.get_open_error()
	file.store_string(JSON.stringify(payload))
	file.flush()
	var write_error := file.get_error()
	file.close()
	return write_error
