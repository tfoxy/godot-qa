extends Control

@onready var hero_panel: PanelContainer = $Background/Margin/Layout/HeroPanel
@onready var title_label: Label = $Background/Margin/Layout/HeroPanel/Margin/Content/TitleLabel
@onready var description_label: Label = $Background/Margin/Layout/HeroPanel/Margin/Content/DescriptionLabel
@onready var language_button: Button = $Background/Margin/Layout/Actions/LanguageButton
@onready var apply_button: Button = $Background/Margin/Layout/Actions/ApplyButton
@onready var back_button: Button = $Background/Margin/Layout/Actions/BackButton


func _ready() -> void:
	set_meta("qa_id", "settings_menu")
	hero_panel.set_meta("qa_id", "settings_panel")
	title_label.set_meta("qa_id", "settings_title")
	description_label.set_meta("qa_id", "settings_description")
	language_button.set_meta("qa_id", "language_button")
	apply_button.set_meta("qa_id", "apply_button")
	back_button.set_meta("qa_id", "back_button")
	back_button.pressed.connect(_return_to_main_menu)
	language_button.grab_focus()


func _return_to_main_menu() -> void:
	get_tree().change_scene_to_file("res://ui/main_menu.tscn")
