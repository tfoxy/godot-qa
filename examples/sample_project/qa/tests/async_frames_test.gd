extends RefCounted


func run_qa_test() -> Dictionary:
	var tree: SceneTree = Engine.get_main_loop() as SceneTree
	var frames_before := Engine.get_process_frames()
	for _index in range(3):
		await tree.process_frame
	var frames_advanced := Engine.get_process_frames() - frames_before
	var failures: Array[Dictionary] = []
	if frames_advanced < 3:
		failures.append({
			"message": "process frames did not advance while awaiting the SceneTree",
			"expected": {"min_process_frames": 3},
			"actual": {"process_frames": frames_advanced},
		})
	return {
		"ok": failures.is_empty(),
		"failures": failures,
		"data": {
			"fixture": "async_frames_test",
			"frames_advanced": frames_advanced,
		},
	}
