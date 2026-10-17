extends RefCounted

const ENABLE_SETTING := "godot_qa/enabled"


# Warm-session-aware replacement for OS.get_cmdline_user_args().
#
# A game that branches on QA project_args (RNG seed, fixture/roster selection,
# test_mode) should read them through this helper so it works under BOTH the
# cold path (one process per scenario) and a warm session (one process, many
# scenarios). When a warm session is active the GodotQaBridge autoload holds the
# CURRENT scenario's args; otherwise this falls back to the process cmdline, so
# cold behaviour is byte-for-byte unchanged.
#
# Opt-in is a one-line change at the single read site:
#   OS.get_cmdline_user_args()  ->  QaArgs.effective_user_args()
# See specs/S56-scenario-project-args-contract.md (Warm session compatibility).
static func effective_user_args() -> PackedStringArray:
	var loop := Engine.get_main_loop()
	if loop is SceneTree:
		var bridge := (loop as SceneTree).get_root().get_node_or_null("GodotQaBridge")
		if bridge != null and bridge.has_method("current_scenario_user_args"):
			var warm_args = bridge.call("current_scenario_user_args")
			var has_override := (
				bridge.has_method("has_scenario_user_args_override")
				and bool(bridge.call("has_scenario_user_args_override"))
			)
			if warm_args is PackedStringArray and (has_override or not (warm_args as PackedStringArray).is_empty()):
				return warm_args
	return OS.get_cmdline_user_args()


static func parse_user_args(args: PackedStringArray) -> Dictionary:
	var options := {}
	var raw_args: Array[String] = []
	var enabled := false

	for raw_arg in args:
		if raw_arg == "--qa":
			enabled = true
			raw_args.append(raw_arg)
			continue
		if not raw_arg.begins_with("--qa-"):
			continue

		enabled = true
		raw_args.append(raw_arg)

		var trimmed := raw_arg.trim_prefix("--qa-")
		var split_index := trimmed.find("=")
		if split_index == -1:
			options[trimmed.replace("-", "_")] = true
			continue

		var key := trimmed.substr(0, split_index).replace("-", "_")
		var value := trimmed.substr(split_index + 1)
		options[key] = value

	return {
		"enabled": enabled,
		"raw_args": raw_args,
		"options": options,
	}
