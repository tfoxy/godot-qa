extends Control

@onready var title_label: Label = $Canvas/TitleLabel
@onready var instruction_label: Label = $Canvas/InstructionLabel
@onready var text_entry_field: LineEdit = $Canvas/TextEntryField
@onready var result_label: Label = $Canvas/ResultLabel


func _ready() -> void:
	set_meta("qa_id", "text_entry_probe")
	title_label.set_meta("qa_id", "text_entry_title")
	instruction_label.set_meta("qa_id", "text_entry_instruction")
	text_entry_field.set_meta("qa_id", "text_entry_field")
	result_label.set_meta("qa_id", "text_entry_result")
	result_label.text = "ready"
	text_entry_field.text_changed.connect(_on_text_changed)
	text_entry_field.grab_focus()


func _on_text_changed(new_text: String) -> void:
	result_label.text = "text:%s" % new_text
