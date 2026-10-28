# Diagnostics and profiling

Choose the smallest evidence capture that answers the question. Bound durations/frame counts and keep live bridge commands serialized per session. Append `--project <project> --json` to commands below unless already shown; inspect installed command help for specialized parameters.

## Editor and runtime errors

Use `errors` for the structured runtime error buffer and `logs` for captured engine output. Choose diagnostic sources explicitly when current import and saved editor evidence answer different questions:

```bash
godot-qa diagnostics editor --source import --project <project> --json
godot-qa diagnostics editor --source editor --project <project> --json
```

`import` launches a fresh headless import and excludes saved editor diagnostics. `editor` reads the saved bridge artifact without launching Godot; enable the Godot QA plugin and run the selected project from the editor after relevant edits to refresh it. The default `combined` merges both, so a historical editor error can still fail a clean current import. Inspect `data.sources` for project, run, capture time, and `included_in_verdict`. Foreign, malformed, or unowned editor evidence is excluded with an advisory in combined mode and fails editor-only mode. No age cutoff makes an old capture current; preserve its provenance when reporting it. `--strict-warnings` applies to diagnostic warnings, not source-availability advisories.

## CLI/addon content identity

`doctor` compares the installed addon's scripts and root `plugin.cfg` with the content manifest compiled into the CLI. Its identity status is `match`, `mismatch`, or `unknown`; mismatch/unknown warn by default. If a project gate requires the CLI's exact addon content, run `doctor --strict-addon-identity --project <project> --json` to make either status fail. This is independent of `--strict-addon-version`: equal version strings can conceal different builds. Inspect project pins or intentional addon modifications before replacing files. A match describes installed files; it does not attest which revision an already-running Godot session loaded. Restart that session after changing its addon.

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

An empty method list does not mean no scripts executed. Add instrumentation only when the task requires it; report its presence with performance evidence. Screenshots, traces, and audio capture can affect the workload being measured; retain capture settings and use a separate representative profile when investigating their overhead. For a slow regression gate, start with wall time, child execution, and orchestration evidence in [Bundle gates](bundle-gates.md) before instrumenting game code.

For engine-provided Script Functions data, enable the Godot QA plugin, run the project from the editor, open the dedicated **Godot QA Profiler** panel, select **Start**, exercise the workload, then select **Stop & Export**. **Export Latest** writes already collected data. Run `profile editor export` to read that experimental artifact. Capturing only with Godot's built-in Profiler panel does not produce the QA export; the CLI does not drive the editor. Check the artifact's run, capture time, frame/function counts, and warnings before treating it as the requested measurement.

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

## Events and telemetry

- `event trace --target <locator> --provider <method> --frames N` records project-provided events. `event summarize <events.json>` reviews the saved stream.
- `telemetry record --provider <id> --frames N --every N` samples a registered read-only source into JSONL. Legacy sources use `--target <locator> --method <name>` instead of `--provider`.

## Audio

Choose the mode that matches the evidence needed:

- `audio capture --live --seconds N --bus Master --analyze` records a bus in an active session; the default maximum is 60 seconds. It observes playback, including the project's timing and bus routing.
- `audio export --target <locator> --method <qa_method> --args '<json-array>'` calls a project-owned `qa_` export hook in an active session. Inspect the project's actual hook and installed help for its arguments; this does not package an arbitrary saved WAV.
- `audio render --target <locator> --method <qa_method> --seconds N` bakes samples through a project-owned `qa_` voice-fill method, including in a headless session. It tests that synthesis path rather than live playback timing.
- `audio analyze <wav> --spectrogram <png>` analyzes an existing WAV without starting Godot; a spectrogram helps inspect the signal but does not establish perceived quality.

Live playback uses an audio clock that can diverge from simulation advanced with fastest/fixed-FPS pacing. For playback timing or synchronization checks, use a realtime workload and verify the effective session/scenario pacing. See [Scenario authoring](scenario-authoring.md) for pacing choices and [Bundle gates](bundle-gates.md) for warm compatibility; preserve existing pacing for deterministic synthesis checks that do not require realtime playback.

Missing probes or audio capture support must remain explicit limitations. Measurements do not establish perceived sound quality and do not approve audio baselines. Use [Visual and artifact review](visual-and-artifacts.md) to index or export results.

Optional deeper contracts: the profiling/soak, physics, event, telemetry, and audio sections of `docs/cli-spec.md` in a matching godot-qa checkout or distribution.
