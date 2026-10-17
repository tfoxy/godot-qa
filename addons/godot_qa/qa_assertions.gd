extends RefCounted

const QaAssertionsImpl = preload("res://addons/godot_qa/qa_assertions_impl.gd")

var _impl: RefCounted = null


func _init(runtime: RefCounted, root_window: Window, runtime_errors := []) -> void:
	_impl = QaAssertionsImpl.new(runtime, root_window, runtime_errors)


func execute(command_name: String, params := {}) -> Dictionary:
	return await _impl.execute(command_name, params)


func surface_audit_probe(params: Dictionary) -> Dictionary:
	return _impl.surface_audit_probe(params)


func teardown() -> void:
	_impl.teardown()
