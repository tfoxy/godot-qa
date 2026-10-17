# Diagnostics and profiling

Choose the smallest evidence capture that answers the question. Bound durations/frame counts and keep live bridge commands serialized per session. Append `--project <project> --json` to commands below unless already shown; inspect installed command help for specialized parameters.

## Editor and runtime errors

Use `errors` for the structured runtime error buffer and `logs` for captured engine output. `diagnostics editor` reads editor/import evidence; for warnings that occur only when running from the editor, enable the Godot QA plugin, press Run in the editor, then read the artifact. `profile editor export` reads the experimental editor Script Functions profiler artifact; it does not launch or drive the editor.

## Profiles and soaks

```bash
godot-qa scenario profile <scenario> --project <project> --json
godot-qa session profile --project <project> --frames 60 --json
```

Live profiles write `profiles/<profile-id>/profile.json` under the session artifact root; scenario profiles write `profile.json` under the scenario run root. Frame/monitor/node data is available without method instrumentation. `top_script_methods` requires project scripts to instrument selected hot paths:

```gdscript
const QaProfiler = preload("res://addons/godot_qa/qa_profiler.gd")

func _process(_delta: float) -> void:
    QaProfiler.begin("PlayerController._process")
    _update_player()
    QaProfiler.end("PlayerController._process")
```

An empty method list does not mean no scripts executed. Add instrumentation only when the task requires it; report its presence with performance evidence.

For an existing soak spec:

```bash
godot-qa scenario profile <scenario> --project <project> --soak <spec.yaml> --json
godot-qa session profile --project <project> --soak <spec.yaml> --detach --json
godot-qa session profile --project <project> --soak-status <run-id> --json
godot-qa session profile --project <project> --soak-stop <run-id> --json
```

Use either a scenario or a live session as appropriate. Soaks require realtime pacing: live sessions must have no fastest/fixed-FPS launch pacing and time scale must be 1. Scenario soaks require `time_control: {time_scale: 1.0}` and no `determinism.fixed_fps`. Set the timeout to cover warmup, measurement, and teardown. A stopped/crashed/timed-out soak is incomplete and cannot pass. One soak can run per session; follow its run ID and artifacts rather than launching duplicates.

## Physics, surfaces, and traces

```bash
godot-qa physics inspect qa:player --project <project> --radius 300 --frames 2 --json
godot-qa trace record --project <project> --target qa:player --frames 120 --sample-every-frames 1 --physics --json
```

Traces record observed frame-by-frame state, not an input replay, under `traces/<trace-id>/trace.json` in the session root. `physics overlay <target> --out <png>` creates artifact-only collider outlines, live or from saved scenario/snapshot evidence. `physics collision-report <target>` or `--from-trace <trace.json>` summarizes collision behavior; it fails only when explicit thresholds or `--strict-warnings` demand it.

`surface audit <provider>` requires a project surface probe and exactly one mode: center/region size, route, or mirrored regions. Use [Runtime instrumentation](runtime-instrumentation.md) when a semantic provider is needed instead of inferring surface behavior from visuals alone.

## Events, telemetry, and audio

- `event trace --target <locator> --provider <method> --frames N` records project-provided events. `event summarize <events.json>` reviews the saved stream.
- `telemetry record --provider <id> --frames N --every N` samples a registered read-only source into JSONL. Legacy sources use `--target <locator> --method <name>` instead of `--provider`.
- `audio capture --live --seconds N --bus Master --analyze` captures bounded live audio; default maximum is 60 seconds. `audio export` handles existing capture evidence. `audio render` performs a headless offline bake through a project `qa_` voice-fill method. `audio analyze <wav> --spectrogram <png>` analyzes saved audio.

Missing probes or audio capture support must remain explicit limitations. Measurements do not establish perceived sound quality and do not approve audio baselines. Use [Visual and artifact review](visual-and-artifacts.md) to index or export results.

Optional deeper contracts: the profiling/soak, physics, event, telemetry, and audio sections of `docs/cli-spec.md` in a matching godot-qa checkout or distribution.
