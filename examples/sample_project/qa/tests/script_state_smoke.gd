extends RefCounted


func run_qa_test() -> Dictionary:
	var state := {
		"fixture": "script_state_smoke",
		"independent_marker": "state-fixture-ran",
		"counter": 1,
	}
	return {
		"ok": true,
		"failures": [],
		"data": state,
	}
