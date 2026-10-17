extends Node

const QaProtocol = preload("res://addons/godot_qa/qa_protocol.gd")

## Milliseconds the serve loop sleeps per frame while no command is in flight.
## Deliberately host-local: `Engine.max_fps` and `OS.low_processor_usage_mode`
## are batch-global engine state the scenario executor snapshots and guards
## against contamination, so this host must never touch them.
const IDLE_FRAME_DELAY_MSEC := 1

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
	_in_flight += 1
	await _handle_connection(peer)
	_in_flight = maxi(_in_flight - 1, 0)


## Runs `_handle_file_request` while the host counts it as in flight.
func _track_file_request(file_name: String) -> void:
	_in_flight += 1
	await _handle_file_request(file_name)
	_in_flight = maxi(_in_flight - 1, 0)


func _handle_connection(peer: StreamPeerTCP) -> void:
	var request_text := await _read_request_line(peer)
	var response := {}
	if request_text == "":
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
		if not FileAccess.file_exists("%s/%s" % [_file_requests_dir, file_name]):
			stale.append(file_name)
	for file_name in stale:
		_processed_file_requests.erase(file_name)


func _handle_file_request(file_name: String) -> void:
	var request_path := "%s/%s" % [_file_requests_dir, file_name]
	var response_path := "%s/%s" % [_file_responses_dir, file_name]
	var response_tmp_path := "%s.tmp" % response_path
	var request := _read_json_file(request_path)
	var response := {}
	if request.is_empty():
		response = QaProtocol.error("protocol_error", "live session file request was empty", {})
	else:
		response = await _bridge.handle_command(request)
	_write_json_file(response_tmp_path, response)
	var rename_error := DirAccess.rename_absolute(response_tmp_path, response_path)
	if rename_error != OK:
		_write_json_file(response_path, response)
	DirAccess.remove_absolute(request_path)


func _read_request_line(peer: StreamPeerTCP) -> String:
	var buffer := ""
	var started_at := Time.get_ticks_msec()
	while Time.get_ticks_msec() - started_at < 5000:
		peer.poll()
		var available := peer.get_available_bytes()
		if available > 0:
			buffer += peer.get_utf8_string(available)
			var newline_index := buffer.find("\n")
			if newline_index != -1:
				return buffer.substr(0, newline_index)
		if peer.get_status() != StreamPeerTCP.STATUS_CONNECTED and available == 0:
			break
		await get_tree().process_frame
	return buffer.strip_edges()


func _read_json_file(path: String) -> Dictionary:
	var file := FileAccess.open(path, FileAccess.READ)
	if file == null:
		return {}
	var parser := JSON.new()
	var parse_error := parser.parse(file.get_as_text())
	if parse_error != OK or not parser.data is Dictionary:
		return {}
	return parser.data


func _write_json_file(path: String, payload: Dictionary) -> void:
	var file := FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		return
	file.store_string(JSON.stringify(payload))
