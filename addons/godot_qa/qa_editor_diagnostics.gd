@tool
extends RefCounted

const CAPTURE_METHOD := "editor_plugin_debugger_errors_panel"
const DIAGNOSTIC_SOURCE := "editor_bridge"
const POLL_SECONDS := 45.0
const POLL_INTERVAL_SECONDS := 1.0

var _editor_interface = null
var _run_id := ""
var _poll_remaining := 0.0
var _poll_interval_remaining := 0.0
var _last_signature := ""


func configure(editor_interface) -> void:
	_editor_interface = editor_interface


func begin_run() -> void:
	_run_id = _make_run_id()
	_last_signature = ""
	_poll_remaining = POLL_SECONDS
	_poll_interval_remaining = 0.0
	_write_capture("editor_project_run_started")


func poll(delta: float) -> void:
	if _poll_remaining <= 0.0:
		return
	_poll_remaining = maxf(0.0, _poll_remaining - delta)
	_poll_interval_remaining -= delta
	if _poll_interval_remaining > 0.0:
		return
	_poll_interval_remaining = POLL_INTERVAL_SECONDS
	_write_capture("editor_debugger_poll")


func _write_capture(reason: String) -> void:
	if _run_id == "":
		_run_id = _make_run_id()
	var diagnostics := _collect_editor_diagnostics()
	var signature := JSON.stringify(diagnostics)
	if signature == _last_signature:
		return
	_last_signature = signature

	var root_dir := _bridge_root_abs()
	var run_root := "%s/%s" % [root_dir, _run_id]
	var diagnostics_path := "%s/diagnostics.json" % run_root
	var latest_path := "%s/latest.json" % root_dir
	DirAccess.make_dir_recursive_absolute(run_root)
	var payload := {
		"capture_method": CAPTURE_METHOD,
		"run_id": _run_id,
		"captured_at": _timestamp_string(),
		"reason": reason,
		"godot_version": str(Engine.get_version_info().get("string", "")),
		"project_root": ProjectSettings.globalize_path("res://"),
		"diagnostic_count": diagnostics.size(),
		"warning_count": _count_severity(diagnostics, "warning"),
		"error_count": _count_severity(diagnostics, "error"),
		"artifacts": {
			"run_root": run_root,
			"diagnostics": diagnostics_path,
			"latest": latest_path,
		},
		"diagnostics": diagnostics,
	}
	_write_json(diagnostics_path, payload)
	_write_json(latest_path, payload)


func _collect_editor_diagnostics() -> Array[Dictionary]:
	var base_control = _editor_base_control()
	if base_control == null:
		return []
	var diagnostics: Array[Dictionary] = []
	_scan_node(base_control, diagnostics)
	return _dedupe_diagnostics(diagnostics)


func _editor_base_control():
	if _editor_interface != null and _editor_interface.has_method("get_base_control"):
		return _editor_interface.call("get_base_control")
	return null


func _scan_node(node: Node, diagnostics: Array[Dictionary]) -> void:
	if node is Tree:
		_scan_tree(node, diagnostics)
	for child in node.get_children():
		if child is Node:
			_scan_node(child, diagnostics)


func _scan_tree(tree: Tree, diagnostics: Array[Dictionary]) -> void:
	var root := tree.get_root()
	if root == null:
		return
	_scan_tree_item(tree, root, diagnostics)


func _scan_tree_item(tree: Tree, item: TreeItem, diagnostics: Array[Dictionary]) -> void:
	var own_lines := _item_text_lines(tree, item)
	var own_text := "\n".join(own_lines)
	if _looks_like_primary_diagnostic(own_text):
		var block_lines := own_lines.duplicate()
		var child := item.get_first_child()
		while child != null:
			var child_lines := _item_text_lines(tree, child)
			var child_text := "\n".join(child_lines)
			if _looks_like_detail_line(child_text):
				block_lines.append_array(child_lines)
			child = child.get_next()
		var diagnostic := _normalize_diagnostic_block(block_lines)
		if not diagnostic.is_empty():
			diagnostics.append(diagnostic)

	var child := item.get_first_child()
	while child != null:
		_scan_tree_item(tree, child, diagnostics)
		child = child.get_next()


func _item_text_lines(tree: Tree, item: TreeItem) -> Array[String]:
	var lines: Array[String] = []
	var column_count := max(1, int(tree.columns))
	for column in range(column_count):
		var text := str(item.get_text(column)).strip_edges()
		if text != "":
			lines.append(text)
	return lines


func _normalize_diagnostic_block(lines: Array) -> Dictionary:
	var cleaned: Array[String] = []
	for raw_line in lines:
		var line := str(raw_line).strip_edges()
		if line != "":
			cleaned.append(line)
	if cleaned.is_empty():
		return {}

	var severity := ""
	var code := ""
	var message := ""
	var script := ""
	var line_number = null
	var column_number = null

	for index in range(cleaned.size()):
		var line := cleaned[index]
		if line.contains("GDScript::reload:"):
			message = _message_after(line, "GDScript::reload:")
		elif line.begins_with("SCRIPT ERROR:") or line.begins_with("ERROR:"):
			severity = "error"
			message = _message_after(line, ":")
		elif line.begins_with("SCRIPT WARNING:") or line.begins_with("WARNING:"):
			severity = "warning"
			message = _message_after(line, ":")
		elif line.begins_with("<GDScript Error>"):
			code = _value_after_or_next(cleaned, index, "<GDScript Error>")
		elif line.begins_with("<GDScript Source>"):
			var location := _value_after_or_next(cleaned, index, "<GDScript Source>")
			var parsed := _parse_source_location(location)
			script = str(parsed.get("script", script))
			line_number = parsed.get("line", line_number)
			column_number = parsed.get("column", column_number)
		elif line.begins_with("at:"):
			var parsed_at := _parse_source_location(_location_in_parentheses(line))
			script = str(parsed_at.get("script", script))
			line_number = parsed_at.get("line", line_number)
			column_number = parsed_at.get("column", column_number)

	if message == "" and code == "":
		return {}
	if severity == "":
		severity = _severity_for_code_message(code, message)
	if code == "":
		code = _code_for_warning_message(message)
	return {
		"severity": severity,
		"code": code if code != "" else null,
		"message": message,
		"script": script if script != "" else null,
		"line": line_number,
		"column": column_number,
		"source": DIAGNOSTIC_SOURCE,
		"raw": "\n".join(cleaned),
	}


func _looks_like_primary_diagnostic(text: String) -> bool:
	var lower := text.to_lower()
	return lower.contains("gdscript::reload:") \
		or lower.contains("script error:") \
		or lower.contains("script warning:") \
		or lower.contains("error:") and lower.contains("script") \
		or _looks_like_warning_message(text)


func _looks_like_detail_line(text: String) -> bool:
	return text.contains("<GDScript Error>") \
		or text.contains("<GDScript Source>") \
		or text.begins_with("at:")


func _looks_like_warning_message(text: String) -> bool:
	var lower := text.to_lower()
	return lower.contains("shadowing") \
		or lower.contains("same name as a built-in function") \
		or lower.contains("same name as a global class") \
		or lower.contains("is never used") \
		or lower.contains("integer division") \
		or lower.contains("integer used when an enum value is expected")


func _severity_for_code_message(code: String, message: String) -> String:
	if code in [
		"SHADOWED_VARIABLE",
		"SHADOWED_GLOBAL_IDENTIFIER",
		"UNUSED_PARAMETER",
		"INTEGER_DIVISION",
		"INT_AS_ENUM_WITHOUT_CAST",
		"NARROWING_CONVERSION",
	] or _looks_like_warning_message(message):
		return "warning"
	return "error"


func _code_for_warning_message(message: String) -> String:
	var lower := message.to_lower()
	if lower.contains("integer division"):
		return "INTEGER_DIVISION"
	if lower.contains("integer used when an enum value is expected"):
		return "INT_AS_ENUM_WITHOUT_CAST"
	if lower.contains("same name as a built-in function") or lower.contains("same name as a global class"):
		return "SHADOWED_GLOBAL_IDENTIFIER"
	if lower.contains("is never used"):
		return "UNUSED_PARAMETER"
	if lower.contains("shadowing"):
		return "SHADOWED_VARIABLE"
	return ""


func _value_after_or_next(lines: Array[String], index: int, prefix: String) -> String:
	var value := lines[index].trim_prefix(prefix).strip_edges()
	if value != "":
		return value
	if index + 1 < lines.size():
		return lines[index + 1].strip_edges()
	return ""


func _message_after(line: String, marker: String) -> String:
	var offset := line.find(marker)
	if offset < 0:
		return line.strip_edges()
	return line.substr(offset + marker.length()).strip_edges()


func _location_in_parentheses(line: String) -> String:
	var start := line.rfind("(")
	if start < 0:
		return line
	var tail := line.substr(start + 1)
	var end := tail.find(")")
	if end >= 0:
		return tail.substr(0, end).strip_edges()
	return tail.strip_edges()


func _parse_source_location(raw_location: String) -> Dictionary:
	var location := raw_location.split("@")[0].strip_edges().trim_suffix(")")
	if location == "":
		return {}
	var last_colon := location.rfind(":")
	if last_colon < 0:
		return {"script": location}
	var head := location.substr(0, last_colon)
	var tail := location.substr(last_colon + 1)
	if not tail.is_valid_int():
		return {"script": location}
	var last_number := int(tail)
	var previous_colon := head.rfind(":")
	if previous_colon >= 0:
		var possible_line := head.substr(previous_colon + 1)
		if possible_line.is_valid_int():
			return {
				"script": head.substr(0, previous_colon),
				"line": int(possible_line),
				"column": last_number,
			}
	return {
		"script": head,
		"line": last_number,
	}


func _dedupe_diagnostics(diagnostics: Array[Dictionary]) -> Array[Dictionary]:
	var seen := {}
	var deduped: Array[Dictionary] = []
	for diagnostic in diagnostics:
		var key := "%s|%s|%s|%s|%s|%s" % [
			str(diagnostic.get("severity", "")),
			str(diagnostic.get("code", "")),
			str(diagnostic.get("message", "")),
			str(diagnostic.get("script", "")),
			str(diagnostic.get("line", "")),
			str(diagnostic.get("column", "")),
		]
		if seen.has(key):
			continue
		seen[key] = true
		deduped.append(diagnostic)
	return deduped


func _count_severity(diagnostics: Array[Dictionary], severity: String) -> int:
	var count := 0
	for diagnostic in diagnostics:
		if str(diagnostic.get("severity", "")) == severity:
			count += 1
	return count


func _write_json(path: String, payload: Dictionary) -> void:
	var file := FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		push_warning("[godot-qa] unable to write editor diagnostics artifact: %s" % path)
		return
	file.store_string(JSON.stringify(payload, "\t"))


func _bridge_root_abs() -> String:
	return ProjectSettings.globalize_path("res://.qa/artifacts/editor-diagnostics")


func _make_run_id() -> String:
	return "%d-%d" % [int(Time.get_unix_time_from_system() * 1000.0), OS.get_process_id()]


func _timestamp_string() -> String:
	var now := Time.get_datetime_dict_from_system(true)
	return "%04d-%02d-%02dT%02d:%02d:%02dZ" % [
		int(now.get("year", 0)),
		int(now.get("month", 0)),
		int(now.get("day", 0)),
		int(now.get("hour", 0)),
		int(now.get("minute", 0)),
		int(now.get("second", 0)),
	]
