extends SceneTree

const Snapshot = preload("res://addons/godot_qa/qa_snapshot.gd")

var checks: Array = []
var failures: Array = []


func check(name: String, condition: bool, actual: Variant) -> void:
	checks.append(name)
	if not condition:
		failures.append({"name": name, "actual": str(actual)})


func _initialize() -> void:
	call_deferred("run")


func run() -> void:
	var subject := Node.new()
	root.add_child(subject)
	var case := str(OS.get_cmdline_user_args()[0])
	var inputs: Array = []
	var expected: Dictionary
	if case == "integer_keys":
		inputs = [{1: "one", 2: {3: ["nested", {4: "four"}]}}]
		expected = {"1": "one", "2": {"3": ["nested", {"4": "four"}]}}
	else:
		inputs = [{1: "integer", "1": "string", 2: "two"}, {2: "two", "1": "string", 1: "integer"}]
		expected = {"1": "string", "2": "two"}
	for input in inputs:
		subject.set_meta("payload", input)
		var tree := Snapshot.node_tree_to_dict(subject)
		var actual: Variant = tree.get("root", {}).get("metadata", {}).get("payload")
		check("metadata values use original keys", actual is Dictionary and actual == expected, tree)
		check("normalized keys sorted", actual is Dictionary and actual.keys() == ["1", "2"], actual)
		var decoded: Variant = JSON.parse_string(JSON.stringify(tree))
		check("snapshot JSON preserves normalized metadata", decoded is Dictionary and decoded.get("root", {}).get("metadata", {}).get("payload") == expected, decoded)
	var report := FileAccess.open("res://report.json", FileAccess.WRITE)
	report.store_string(JSON.stringify({"checks": checks, "failures": failures}))
	report.close()
	quit(0 if failures.is_empty() else 1)
