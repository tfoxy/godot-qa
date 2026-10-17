extends PanelContainer

@onready var title_label: Label = $Panel/Margin/Buttons/TitleLabel
@onready var resume_button: Button = $Panel/Margin/Buttons/ResumeButton
@onready var quit_button: Button = $Panel/Margin/Buttons/QuitButton


func _ready() -> void:
	set_meta("qa_id", "pause_menu")
	title_label.set_meta("qa_id", "pause_title")
	resume_button.set_meta("qa_id", "resume_button")
	quit_button.set_meta("qa_id", "quit_button")
