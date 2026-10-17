extends RefCounted


func run_qa_test() -> Dictionary:
	OS.delay_msec(150)
	return {
		"ok": true,
		"failures": [],
		"data": {
			"fixture": "slow_manifest_test",
			"delay_ms": 150,
		},
	}
