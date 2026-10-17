extends SceneTree

# Cold, isolated one-shot scenario/check runner.
#
# Launched via `--script res://addons/godot_qa/qa_runner.gd` with a
# `--qa-run-request=<abs-path>` (scenario) or `--qa-check-request=<abs-path>`
# (check). All scenario/check execution lives in QaScenarioExecutor, which is
# shared verbatim with the warm in-session path (qa_bridge.gd) so cold and warm
# produce byte-identical result.json. This file only owns the one-shot concerns:
# parsing the cmdline request path, printing the JSON envelope to stdout, and
# quitting the process with the right exit code.

const QaArgs = preload("res://addons/godot_qa/qa_args.gd")
const QaProtocol = preload("res://addons/godot_qa/qa_protocol.gd")
const QaScenarioExecutor = preload("res://addons/godot_qa/qa_scenario_executor.gd")


func _initialize() -> void:
	var parsed_args := QaArgs.parse_user_args(OS.get_cmdline_user_args())
	var request_path = str(parsed_args.get("options", {}).get("run_request", ""))
	var check_request_path = str(parsed_args.get("options", {}).get("check_request", ""))
	if request_path == "" and check_request_path == "":
		print(JSON.stringify(QaProtocol.error(
			"missing_request",
			"qa_runner requires --qa-run-request=<abs-path> or --qa-check-request=<abs-path>",
			{"qa_args": parsed_args},
		)))
		quit(1)
		return

	var executor = QaScenarioExecutor.new(self)
	var exit_code := 0
	if check_request_path != "":
		exit_code = await executor.run_check(check_request_path)
	else:
		var outcome: Dictionary = await executor.run_scenario(request_path)
		exit_code = _emit_scenario_outcome(outcome)
	quit(exit_code)


# Reproduce the legacy stdout contract: request errors and write failures print a
# QaProtocol.error envelope; success prints the result payload. Returns the
# process exit code.
func _emit_scenario_outcome(outcome: Dictionary) -> int:
	if str(outcome.get("status", "")) == "request_error":
		print(JSON.stringify(outcome.get("error", {})))
		return int(outcome.get("exit_code", 1))
	if not bool(outcome.get("write_ok", true)):
		print(JSON.stringify(QaProtocol.error(
			"result_write_failed",
			"qa_runner could not write result.json",
			{"result_path": outcome.get("result_path", "")},
		)))
		return 1
	print(JSON.stringify(outcome.get("payload", {})))
	return int(outcome.get("exit_code", 0))
