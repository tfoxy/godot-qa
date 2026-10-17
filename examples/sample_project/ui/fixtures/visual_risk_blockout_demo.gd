extends Control


func _ready() -> void:
	set_meta("qa_id", "visual_risk_blockout_demo")
	$Canvas/BlockoutPanel.set_meta("qa_id", "visual_risk_blockout_panel")
	$Canvas/DebugPrimaryStrip.set_meta("qa_id", "visual_risk_debug_primary_strip")
