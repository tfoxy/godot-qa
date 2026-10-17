@tool
extends EditorPlugin

const QaEditorDiagnostics = preload("res://addons/godot_qa/qa_editor_diagnostics.gd")
const QaEditorProfiler = preload("res://addons/godot_qa/qa_editor_profiler.gd")
const QaEditorProfilerDebugger = preload("res://addons/godot_qa/qa_editor_profiler_debugger.gd")

const AUTOLOAD_NAME := "GodotQaBridge"
const AUTOLOAD_PATH := "res://addons/godot_qa/qa_bridge.gd"

var _editor_diagnostics: RefCounted = null
var _editor_profiler: RefCounted = null
var _editor_profiler_debugger = null
var _editor_profiler_panel: Control = null
var _editor_profiler_status_label: Label = null

func _enter_tree() -> void:
	var autoload_setting := "autoload/%s" % AUTOLOAD_NAME
	if not ProjectSettings.has_setting(autoload_setting):
		add_autoload_singleton(AUTOLOAD_NAME, AUTOLOAD_PATH)
	_editor_profiler_debugger = QaEditorProfilerDebugger.new()
	add_debugger_plugin(_editor_profiler_debugger)
	_editor_diagnostics = QaEditorDiagnostics.new()
	if _editor_diagnostics.has_method("configure"):
		_editor_diagnostics.call("configure", get_editor_interface())
	_editor_profiler = QaEditorProfiler.new()
	if _editor_profiler.has_method("configure"):
		_editor_profiler.call("configure", get_editor_interface(), _editor_profiler_debugger)
	_add_editor_profiler_panel()
	set_process(true)

func _exit_tree() -> void:
	set_process(false)
	_remove_editor_profiler_panel()
	if _editor_profiler_debugger != null:
		remove_debugger_plugin(_editor_profiler_debugger)
	_editor_profiler = null
	_editor_profiler_debugger = null
	_editor_diagnostics = null
	var autoload_setting := "autoload/%s" % AUTOLOAD_NAME
	if ProjectSettings.has_setting(autoload_setting):
		remove_autoload_singleton(AUTOLOAD_NAME)

func _build() -> bool:
	if _editor_diagnostics != null and _editor_diagnostics.has_method("begin_run"):
		_editor_diagnostics.call("begin_run")
	return true

func _process(delta: float) -> void:
	if _editor_diagnostics != null and _editor_diagnostics.has_method("poll"):
		_editor_diagnostics.call("poll", delta)
	if _editor_profiler != null and _editor_profiler.has_method("poll"):
		_editor_profiler.call("poll", delta)
	_update_editor_profiler_status()


func _add_editor_profiler_panel() -> void:
	var panel := VBoxContainer.new()
	panel.name = "Godot QA Profiler"
	panel.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	panel.size_flags_vertical = Control.SIZE_EXPAND_FILL

	var toolbar := HBoxContainer.new()
	toolbar.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	panel.add_child(toolbar)

	var start_button := Button.new()
	start_button.text = "Start"
	start_button.tooltip_text = "Start the Godot Editor servers profiler and capture raw Script Functions messages."
	start_button.pressed.connect(_on_editor_profiler_start_pressed)
	toolbar.add_child(start_button)

	var stop_button := Button.new()
	stop_button.text = "Stop & Export"
	stop_button.tooltip_text = "Stop the profiler and write the latest Godot QA editor profiler artifact."
	stop_button.pressed.connect(_on_editor_profiler_stop_pressed)
	toolbar.add_child(stop_button)

	var export_button := Button.new()
	export_button.text = "Export Latest"
	export_button.tooltip_text = "Write the latest captured Script Functions profiler artifact."
	export_button.pressed.connect(_on_editor_profiler_export_pressed)
	toolbar.add_child(export_button)

	_editor_profiler_status_label = Label.new()
	_editor_profiler_status_label.text = "Idle"
	_editor_profiler_status_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	toolbar.add_child(_editor_profiler_status_label)

	_editor_profiler_panel = panel
	add_control_to_bottom_panel(panel, "Godot QA Profiler")


func _remove_editor_profiler_panel() -> void:
	if _editor_profiler_panel != null:
		remove_control_from_bottom_panel(_editor_profiler_panel)
		_editor_profiler_panel.queue_free()
	_editor_profiler_panel = null
	_editor_profiler_status_label = null


func _on_editor_profiler_start_pressed() -> void:
	if _editor_profiler != null and _editor_profiler.has_method("begin_capture"):
		_editor_profiler.call("begin_capture")
	_update_editor_profiler_status()


func _on_editor_profiler_stop_pressed() -> void:
	if _editor_profiler != null and _editor_profiler.has_method("stop_and_export"):
		_editor_profiler.call("stop_and_export")
	_update_editor_profiler_status()


func _on_editor_profiler_export_pressed() -> void:
	if _editor_profiler != null and _editor_profiler.has_method("export_latest"):
		_editor_profiler.call("export_latest")
	_update_editor_profiler_status()


func _update_editor_profiler_status() -> void:
	if _editor_profiler_status_label == null or _editor_profiler == null:
		return
	if _editor_profiler.has_method("status_text"):
		_editor_profiler_status_label.text = str(_editor_profiler.call("status_text"))
