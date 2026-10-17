extends RefCounted


func run_qa_test() -> Dictionary:
	while true:
		OS.delay_msec(1000)
	return {
		"ok": true,
		"failures": [],
	}
