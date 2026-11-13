extends SceneTree


func _initialize() -> void:
	var cases: Array = JSON.parse_string(FileAccess.get_file_as_string("res://cases.json"))
	for case: Dictionary in cases:
		var decoded: Variant = bytes_to_var(PackedByteArray(case.bytes))
		var expected := NodePath(str(case.path))
		if (
			not decoded is NodePath
			or decoded != expected
			or decoded.get_name_count() != int(case.names)
			or decoded.get_subname_count() != int(case.subnames)
			or decoded.is_absolute() != expected.is_absolute()
		):
			push_error("NodePath round trip failed: %s -> %s" % [case.path, decoded])
			quit(1)
			return
	print("NodePath round trips: %d passed" % cases.size())
	quit(0)
