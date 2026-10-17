extends Control

var _emitted := false


func _ready() -> void:
	set_meta("qa_id", "runtime_error_probe")
	call_deferred("_emit_runtime_error_when_ready")


func _emit_runtime_error_when_ready() -> void:
	while not _emitted:
		var bridge := get_node_or_null("/root/GodotQaBridge")
		if bridge != null and bridge.has_method("record_runtime_error"):
			bridge.call(
				"record_runtime_error",
				"Runtime error probe fired",
				"fixture",
				{
					"scene": "res://ui/fixtures/runtime_error_probe.tscn",
					"qa_id": "runtime_error_probe",
				},
			)
			_emitted = true
			return
		await get_tree().process_frame
