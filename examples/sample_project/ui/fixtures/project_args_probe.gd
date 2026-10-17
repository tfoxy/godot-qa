extends Control

const QaArgs = preload("res://addons/godot_qa/qa_args.gd")

var _raw_user_args: Array[String] = []
var _filtered_project_args: Array[String] = []
var _qa_control_args: Array[String] = []


func _ready() -> void:
	set_meta("qa_id", "project_args_probe")
	_capture_project_args()


func get_project_args_qa_state() -> Dictionary:
	return {
		"raw_user_args": _raw_user_args,
		"project_args": _filtered_project_args,
		"raw_count": _raw_user_args.size(),
		"project_arg_count": _filtered_project_args.size(),
		"qa_control_count": _qa_control_args.size(),
		"observed_qa_separator_side": _qa_control_args.has("--qa"),
	}


func _capture_project_args() -> void:
	_raw_user_args.clear()
	_filtered_project_args.clear()
	_qa_control_args.clear()

	# Warm-session-aware: returns the current scenario's QA args under a warm
	# session, else the process cmdline (identical to OS.get_cmdline_user_args()
	# on the cold path).
	for arg in QaArgs.effective_user_args():
		var text := str(arg)
		_raw_user_args.append(text)
		if text == "--qa" or text.begins_with("--qa-"):
			_qa_control_args.append(text)
			continue
		_filtered_project_args.append(text)
