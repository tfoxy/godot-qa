extends RefCounted

# Deliberately mutates Engine.time_scale and Engine.max_fps without restoring
# them, so batch mode's per-test engine-settings-drift contamination check has
# a multi-field fixture to catch. Tagged "diagnostic" so it never runs as part
# of "smoke"/"full".


func run_qa_test() -> Dictionary:
	Engine.time_scale = 3.0
	Engine.max_fps = 123
	return {
		"ok": true,
		"failures": [],
		"data": {
			"fixture": "script_engine_drift",
		},
	}
