# CLI Specification

All commands used by agents must support `--json`. JSON output must be stable enough for automation.

The public CLI contract is implementation-agnostic. The shipped CLI implementation is now the Rust binary, and this document remains the stable contract the implementation must preserve.

Compatibility requirements:

- preserve the shipped leaf command names and flag structure documented here
- preserve the top-level JSON result envelope keys: `ok`, `command`, `message`, `data`, `failures`
- preserve failure payload field names and meanings when present, including `type`, `command`, `path`, `expected`, `actual`, `target`, `severity`, and `artifacts`
- preserve exit behavior: `0` for success, `1` for validation/runtime/contract failures, `2` for parse/usage failures
- preserve artifact directory ownership and layout semantics
- preserve live runtime protocol command names plus request/response structure

Human-readable formatting may evolve; machine-readable contracts remain stable.

`doctor` discovers source-addon metadata beside the running distribution or in a development checkout, never from the compiler build path. When source-addon metadata is absent, the selected project addon remains authoritative.

## Global options

```bash
godot-qa [--project <path>] [--godot-bin <path>] [--artifact-dir <path>] [--json] [--verbose] <command> ...
```

Global flags:

```text
--project <path>        Godot project root or repo root
--godot-bin <path>      Godot executable override
--artifact-dir <path>   Artifact output override (relative paths resolve from the selected project root)
--json                  Print stable machine-readable JSON output
--verbose               Include command-specific diagnostic details under `data.diagnostics`
```

Still planned, not implemented in S1:

```text
--quiet                 Minimize human output
```

## Project Configuration

Projects may define optional `godot-qa` settings in `qa/config.yaml`. Projects may also define uncommitted local overrides in `qa/config.local.yaml`.

```yaml
godot:
  launch_prefix:
    - env
    - GODOT_QA=1
  windowed_launch_prefix:
    - xvfb-run
    - -a
  args:
    - --rendering-driver
    - opengl3
  project_args:
    - --no-countdown
  launch_mode: headless
scenario:
  warm: true
  warm_strict: true
  schedule: longest-first
artifacts:
  keep_runs: 20
logs:
  ignore:
    - ObjectDB instances leaked at exit
```

`godot.launch_prefix` is a generic argv prefix applied to Godot launches: imports, checks, one-shot scenarios, generated one-shot assertions, layout audits, recorder sessions, live sessions, and patch application. `godot.windowed_launch_prefix` is an additional argv prefix used only when the selected launch mode is windowed. For headless launches the effective command is `launch_prefix + selected Godot binary + generated/config/command Godot args`; for windowed launches it is `launch_prefix + windowed_launch_prefix + auto-detected virtual display prefix + selected Godot binary + generated/config/command Godot args`.
`godot.args` is a project-wide argv array passed to Godot before Godot's `--` separator, after generated CLI/display args and before scenario-specific `godot_args`. `godot.project_args` is a project-wide argv array passed after generated QA bridge args and before scenario-specific or command-specific project args. `godot.launch_mode` may be `auto`, `headless`, or `windowed`; explicit CLI `--headless`/`--windowed` flags take precedence, then config launch mode, then automatic mode. The audio driver is an independent axis: `godot.audio_mode` may be `auto`, `on`, or `off`, and explicit CLI `--audio`/`--no-audio` flags take precedence, then config audio mode, then automatic mode — mirroring the display axis.
Automatic launch mode uses headless for ordinary state/logic scenarios and `audio_export`. It uses windowed when a scenario needs rendered media evidence: saved screenshot capture, `screenshot_match`, screenshot-backed visual assertions, and layout/viewport matrix runs that capture rendered output. Automatic audio mode is independent of the display: it uses `--audio-driver Dummy` for scenarios that don't capture audio and the real audio driver only when a scenario uses `audio_capture`; `scenario run --audio` / `--no-audio` and `godot.audio_mode: auto|on|off` override it, mirroring `--headless`/`--windowed` and `godot.launch_mode`.
`scenario.schedule` selects the bundle dispatch order (default `longest-first`); `manifest` keeps bundle expansion order. `logs.ignore` lists substrings that silence engine `ERROR:`/`WARNING:` lines in every log scan (cold, warm, and the warm session-noise scan); it is project-owned and silences real errors, so fix the game first.

`artifacts.keep_runs` bounds how many run directories are kept under the artifacts root. When it is set to a positive integer `N`, a `scenario run` keeps the newest `N` run directories for that scenario, and a `scenario bundle run` keeps the newest `N` run directories for every scenario it ran and for the bundle itself, then removes abandoned pooled live-session directories under `.qa/artifacts/live/` that started more than one hour before the oldest retained bundle run. That margin exists because a bundle run id records when the run finished while its pooled sessions started when it began, so a bare cutoff would discard the retained run's own session logs. When it is unset there is no limit and nothing is ever pruned, which is the default. Pruning is housekeeping: it runs only after the command's artifacts are written, it never touches the run this invocation just produced, and a deletion error is reported as a warning rather than changing the verdict or the exit code.

When both config files exist, `qa/config.local.yaml` overlays keys from `qa/config.yaml`. Local `godot.launch_prefix`, `godot.windowed_launch_prefix`, `godot.args`, and `godot.project_args` arrays replace the base arrays, and empty local arrays explicitly clear the base defaults. Local `godot.launch_mode` replaces the base launch mode.

Version probing has one narrow exception: `doctor` first tries the configured prefixed probe, but if that prefixed `godot --version` fails, it may retry the selected Godot binary directly as `godot --version`. This fallback is only for version probes, does not bypass an invalid explicit Godot binary path, and does not change normal launch behavior. `doctor --json` reports the probe path under `data.godot.version_probe`, including the attempted prefixed command, whether direct fallback was used, the fallback reason, and the final probe command that produced the version.

Rules:

- Missing `qa/config.yaml` and `qa/config.local.yaml` preserves the default direct Godot launch behavior.
- Only `qa/config.yaml` and `qa/config.local.yaml` are read. Unsupported spellings — `qa/config.json`, `qa/config.local.json`, `qa/config.yml`, and `qa/config.local.yml` — are ignored, but each one that exists on disk surfaces an `unsupported_config_file` warning (`type: "unsupported_config_file"`, `severity: "warning"`, `path`, `expected`, `remediation`) so a project that ships one learns it is being skipped instead of failing silently. The warning appears in `doctor --json` `data.warnings[]` and in the `data.warnings[]` of the Godot-launching commands (`scenario run`, `scenario profile`, `scenario bundle run`, `check`, `import`, `session start`); it never changes exit codes or verdicts.
- `godot.launch_prefix`, `godot.windowed_launch_prefix`, `godot.args`, and `godot.project_args` must be arrays of non-empty strings. They are not shell syntax and are not split or expanded.
- Keys that `godot-qa` does not recognise are ignored, and each one surfaces an `unknown_config_key` warning (`type: "unknown_config_key"`, `severity: "warning"`, `path`, `key` as the dotted config path, `expected` as the known keys at that level, and `remediation`) so a typo like `warmStrict` or a guessed `scenario.parallelism` is visible instead of silently disabling the feature. The warning appears in `doctor --json` `data.warnings[]` and in the `data.warnings[]` of the Godot-launching commands; it never changes exit codes or verdicts. Recognised keys are `godot`, `scenario`, `artifacts`, and `logs` at the root; `launch_prefix`, `windowed_launch_prefix`, `args`, `project_args`, `launch_mode`, and `audio_mode` under `godot`; `warm`, `warm_strict`, `jobs`, and `schedule` under `scenario`; `keep_runs` under `artifacts`; and `ignore` under `logs`.
- `godot.launch_mode` must be `auto`, `headless`, or `windowed`.
- `scenario.schedule` must be `manifest`, `longest-first`, or `longest_first`; any other value fails Godot-launching commands with `type: "invalid_project_config"`.
- `logs.ignore` must be an array of non-empty strings. A local `logs.ignore` array replaces the base array and an empty local array clears it, exactly like `godot.args`.
- `artifacts.keep_runs` must be a positive integer. Unset means unlimited retention. Zero, a negative value, or a non-integer fails Godot-launching commands with `type: "invalid_project_config"` rather than being treated as unlimited.
- Prefix entries that contain a path separator and are relative resolve from the project root. Bare command names resolve through `PATH`.
- `--godot-bin` and `GODOT_BIN` still select the Godot binary; the prefix wraps that selected binary.
- Malformed YAML config or invalid Godot config fields fail Godot-launching commands with `type: "invalid_project_config"`.
- Failure `command` fields report the full effective argv, including the prefix, when a launch is attempted.

## Current Public Command Surface

`docs/cli-spec.md` is the canonical current public CLI inventory. The command paths below are the exhaustive shipped leaf commands today.

```text
doctor
install-skills
import
check
diagnostics editor
profile editor export
scenario list
scenario bundle list
scenario bundle run
scenario lint
scenario run
scenario sweep
scenario profile
scenario init
scenario record
asset try
asset status
asset keep
asset discard
review list-templates
review init
live-gate list
live-gate run
qa-state validate
artifact query
artifact summarize
artifact prune
artifact begin-run
artifact validate-result
artifact export
session start
session status
session list
session run-script
session wait
session time-scale
session reload
session watch
session stop
session pause
session resume
session step
session profile
input tap
input hold
input release
input click
input mouse click
input mouse wheel
input touch down
input touch move
input touch up
input touch tap
input drag
input key
input type
input joy button
input joy motion
snapshot
tree
ui
focus
focus graph
node inspect
physics inspect
physics overlay
physics collision-report
surface audit
trace record
event trace
event summarize
telemetry record
audio capture
audio export
audio render
audio analyze
errors
layout audit
layout audit-current
assert
screenshot capture
screenshot assert-startup-presented
screenshot compare
screenshot sequence
screenshot contact-sheet
visual preflight
visual audit
visual isolate
edit-live set
edit-live call
edit-live diff
edit-live export-patch
apply-patch
mcp
logs
```

## Editor Diagnostics

`diagnostics editor` reads the Godot QA editor bridge artifact when present, launches Godot through the normal process/artifact layer with an editor/import-compatible `--headless --import --log-file` pass, then normalizes and merges editor, import, or GDScript diagnostics into JSON. This command is distinct from live `errors`: `errors` reads the structured QA runtime-error buffer from an active session, while `diagnostics editor` is for editor/import/GDScript analyzer diagnostics.

```bash
godot-qa diagnostics editor --project <project> --json
godot-qa diagnostics editor --project <project> --strict-warnings --json
```

For warnings that only appear in the Godot Editor Debugger panel after pressing Run, enable the `Godot QA` plugin, run the project from the editor, then run `godot-qa diagnostics editor --project <project> --json`. The plugin writes `.qa/artifacts/editor-diagnostics/latest.json` and timestamped `.qa/artifacts/editor-diagnostics/<run-id>/diagnostics.json` files. If that bridge artifact is missing, `diagnostics editor` still returns log-only results with an advisory note.

Successful JSON includes advisory warning facts by default:

```json
{
  "ok": true,
  "command": "diagnostics editor",
  "data": {
    "diagnostic_count": 1,
    "warning_count": 1,
    "error_count": 0,
    "artifacts": {
      "editor_bridge": ".qa/artifacts/editor-diagnostics/latest.json",
      "logs": ".qa/artifacts/diagnostics/<run-id>/logs.txt",
      "raw_godot_log": ".qa/artifacts/diagnostics/<run-id>/godot.log"
    },
    "capture_methods": [
      "editor_plugin_debugger_errors_panel",
      "godot --headless --import --log-file"
    ],
    "diagnostics": [
      {
        "severity": "warning",
        "code": "SHADOWED_VARIABLE",
        "message": "The local function parameter \"device_index\" is shadowing an already-declared variable.",
        "script": "res://src/match_config.gd",
        "line": 464,
        "column": null,
        "source": "logs",
        "raw": "..."
      }
    ]
  },
  "failures": []
}
```

Warnings remain advisory unless `--strict-warnings` is supplied, in which case warning diagnostics produce `type: "editor_diagnostics_warnings_present"` and exit code `1`. Error diagnostics always fail with `type: "editor_diagnostics_errors_present"` when the Godot process itself otherwise exits cleanly. A non-zero Godot process exit fails as `type: "godot_editor_diagnostics_failed"` and preserves the captured log artifacts.

## Experimental Editor Profiler Export

`profile editor export` reads the latest Godot QA editor-plugin profiler artifact and returns the Godot Editor's built-in Script Functions data as structured JSON. It is an editor-side export source, not a live runtime command: the CLI does not launch, focus, or drive the editor in v1.

```bash
godot-qa profile editor export --project <project> --json
```

Workflow:

1. Enable the `Godot QA` plugin in the project.
2. Run the project from the Godot Editor.
3. Open the `Godot QA Profiler` panel.
4. Use `Start`, `Stop & Export`, or `Export Latest`.
5. Run `godot-qa profile editor export --project <project> --json`.

Successful JSON uses the standard result envelope:

```json
{
  "ok": true,
  "command": "profile editor export",
  "data": {
    "source": "editor_script_functions",
    "capture_method": "editor_debugger_raw_servers_profile",
    "experimental": true,
    "frame_count": 12,
    "script_function_count": 1,
    "script_functions": [
      {
        "signature": "res://player.gd::Player::_process",
        "script": "res://player.gd",
        "function": "_process",
        "call_count": 120,
        "total_ms": 14.2,
        "self_ms": 8.1,
        "internal_ms": 0.0,
        "max_total_ms": 3.4
      }
    ],
    "warnings": [],
    "artifacts": {
      "source": ".qa/artifacts/editor-profiler/latest.json",
      "profile": ".qa/artifacts/profile-editor/<run-id>/profile.json",
      "result": ".qa/artifacts/profile-editor/<run-id>/result.json"
    }
  },
  "failures": []
}
```

The exported source is marked `experimental` because it uses Godot editor debugger profiler messages and the source-derived raw frame layout. The supported public control surface is `EditorDebuggerPlugin` plus `EditorDebuggerSession.toggle_profiler("servers", true, [max_funcs, include_native])`; the normalized frame data comes from raw `servers:function_signature`, `servers:profile_frame`, and `servers:profile_total` messages. The plugin does not scrape the visible Profiler table.

Missing editor capture fails with `type: "editor_profile_missing"` and points at `.qa/artifacts/editor-profiler/latest.json`. Malformed artifacts fail with `type: "editor_profile_invalid"` and include the artifact path plus expected/actual schema facts. If the editor plugin cannot connect to raw debugger data, it writes warnings such as `editor_profiler.debug_data_unavailable`; the CLI surfaces those warnings without inventing script-function rows.

### `scenario run` Audio Evidence

`godot-qa scenario run <name> --json` preserves the standard result envelope and may add audio evidence when, and only when, the saved scenario contains explicit `audio_capture` or `audio_export` steps. Normal scenario runs do not record audio by default.

Successful audio steps add bounded fields under `data.audio_artifacts` and `data.audio_analysis` and persist the same evidence in the run's `result.json`. `data.audio_artifacts.<name>.wav` points to `audio/<name>.wav`; `data.audio_artifacts.<name>.analysis` points to `audio/<name>.analysis.json`; `data.audio_analysis.<name>` contains scalar WAV facts such as duration, channel count, sample rate, peak, RMS, silence ratio, clipping count, loop-boundary delta, transient count, zero-crossing rate, and low/mid/high spectral buckets.

Audio failure types are stable and actionable:

- `audio_capture_unavailable`: the requested bus or Godot recording API is unavailable.
- `audio_capture_failed`: capture, directory creation, or WAV write failed.
- `audio_export_rejected`: the target/method is not allowed, including non-`qa_` methods.
- `audio_export_oversized`: the export returned or wrote an empty or oversized payload.
- `audio_export_invalid_return`: the export returned an unsupported value or non-WAV bytes.
- `audio_export_invalid_metadata`: the export returned metadata that cannot be represented as JSON.
- `audio_export_path_rejected`: the export pointed outside the controlled run audio directory.
- `audio_export_failed`: the controlled export directory or artifact write failed.

These failures include `command: "audio_capture"` or `command: "audio_export"` at the runtime failure level, the relevant target or bus, expected and actual values, and run artifact context when available. The top-level CLI exits nonzero for validation/runtime failures. Analysis is intentionally non-subjective: it reports measurable WAV facts and silence/near-silence evidence but does not decide whether a track sounds good or approve audio baselines.

### `audio capture`, `audio analyze`, `audio export`, and `audio render`

Live audio commands are explicit, bounded evidence commands. They do not capture host microphone input, do not run for every live session by default, and do not judge subjective sound quality. Warnings are factual measurements or capability reports; `--strict-warnings` is the opt-in mode that turns warnings into a nonzero validation failure.

```bash
godot-qa audio capture --project <project> --live --seconds 30 --bus Master --name current_race --json
godot-qa audio capture --project <project> --live --seconds 60 --stop-after-state '{"target":"qa:race","source":"method","method":"get_qa_victory_state","matches":[{"path":"victory_elapsed_seconds","operator":"greater_than","expected":10}]}' --json
godot-qa audio capture --project <project> --live --seconds 10 --bus SFX --out .qa/artifacts/manual/sfx_capture.wav --analyze --json
godot-qa audio analyze .qa/artifacts/manual/sfx_capture.wav --json
godot-qa audio export --project <project> --target qa:audio_controller --method qa_export_audio_track --args '["sfx_bus_snapshot"]' --name exported_sfx --analyze --json
godot-qa audio render --project <project> --target qa:audio_controller --method qa_fill_audio --seconds 8 --mix-rate 44100 --name music_bed --analyze --json
godot-qa audio analyze .qa/artifacts/manual/sfx_capture.wav --spectrogram .qa/artifacts/manual/sfx_capture.png --json
```

`audio capture --live` requires an active live session for the selected project. `--setup-script <path>` may be supplied to run a `session run-script` fixture/setup script before capture starts. `--seconds <float>` is required, must be positive, and is capped by the effective live capture limit. The default maximum is 60 seconds; projects or users may lower it, and a larger caller-owned limit must be explicit with `--max-seconds <float>`. JSON reports `requested_seconds`, `effective_seconds`, `max_seconds`, `bus`, `name`, and artifact paths. `--bus <name>` defaults to `Master` when omitted. `--stop-after-state '<json>'` may stop capture early when a read-only `state_matches` predicate becomes true; the JSON object uses `target`, `source`, `method`, and either `matches` or `path`/`operator`/`expected`, with optional `sample_every_frames` defaulting to `1`. If the predicate does not match before the effective duration, the command still writes the WAV evidence and exits nonzero with `audio_capture.stop_condition_timeout`. Bus capture can be unavailable on some runtimes; the command must return structured evidence instead of silently passing.

`--name <artifact-name>` writes under `.qa/artifacts/live/<session-id>/audio/<name>.wav`. The runtime always writes this controlled artifact (reported as `artifacts.wav`); `--out <path>` is a CLI-controlled copy destination written after the runtime returns, so any relative `.wav` path under `.qa/artifacts` is accepted regardless of the session audio directory. The copy is reported as `artifacts.out`, and when `--analyze` is set its analysis JSON is copied alongside as `artifacts.out_analysis`. `--out` must not let Godot write arbitrary host paths. `--analyze` writes `<name>.analysis.json` and includes a bounded summary in `data.analysis`. `audio analyze <wav>` is standalone: it analyzes an existing WAV without starting Godot and writes JSON to stdout unless an artifact path is requested by the command surface.

`audio export` calls project-owned export hooks during an active live session. The target must resolve through the normal live target resolver, and `--method` must begin with `qa_`, such as `qa_export_audio_track`. Arguments are JSON from `--args`. The hook may return bounded RIFF/WAVE bytes/base64 or a JSON-compatible dictionary pointing to a WAV already written under the controlled live audio directory. Non-`qa_` methods, unsupported returns, oversized returns, invalid metadata, and paths outside the controlled audio directory fail with structured details.

`audio render` is the offline-bake counterpart to `audio export`: the *runtime* owns the bake loop and the WAV container while the project supplies only a `qa_` voice-fill method. The command computes a frame count from `--seconds` × `--mix-rate` (capped by `--max-seconds`), calls the resolved `qa_` method with `(frame_count, mix_rate, ...args)`, and packs the returned mono samples (`PackedFloat32Array`, `PackedVector2Array` downmixed, or `Array` of floats in `[-1, 1]`) into a controlled 16-bit WAV under the live audio directory. This works headless because audio is synthesized straight to a buffer — nothing is captured from an output device — so a deterministic music/SFX voice can be graded in CI rather than only judged live. `--out`, `--analyze`, and `--strict-warnings` behave as for `audio export`. Non-`qa_` methods, empty or oversized output, and unsupported return values fail with structured `audio_render.*` details.

`audio analyze --spectrogram <path.png>` additionally renders a waveform + log-frequency spectrogram PNG next to the analysis, so a reviewer can *see* whether the audio clears a non-prototype bar. The path must end in `.png`; a rejected path surfaces an `invalid_params` failure without aborting the rest of the analysis.

Live audio JSON keeps raw samples out of command output. A successful analyzed result uses the normal envelope and includes fields shaped like:

```json
{
  "ok": true,
  "command": "audio capture",
  "data": {
    "artifacts": {
      "wav": "/abs/path/.qa/artifacts/live/<session-id>/audio/current_race.wav",
      "analysis": "/abs/path/.qa/artifacts/live/<session-id>/audio/current_race.analysis.json"
    },
    "requested_seconds": 30.0,
    "effective_seconds": 30.0,
    "max_seconds": 60.0,
    "bus": "Master",
    "analysis": {
      "duration_seconds": 30.0,
      "sample_rate": 48000,
      "channels": 2,
      "peak": 0.82,
      "rms": 0.18,
      "silence_ratio": 0.0,
      "near_silence_ratio": 0.0,
      "clipping_sample_count": 0,
      "clipping_region_count": 0,
      "transient_count": 42,
      "transients_per_second": 1.4,
      "zero_crossing_rate": 0.12,
      "spectral_bands": {
        "low": 0.42,
        "mid": 0.31,
        "high": 0.14
      },
      "spectral_centroid_hz": 1820.0,
      "spectral_flatness": 0.46,
      "air_ratio": 0.32,
      "crest_factor": 4.6,
      "strong_peak_count": 8,
      "window_seconds": 1.0,
      "hop_seconds": 0.5,
      "peak_by_window": [],
      "rms_by_window": [],
      "transients_by_window": [],
      "suspected_event_spam_regions": [],
      "clipping_regions": [],
      "silence_regions": []
    },
    "warnings": []
  }
}
```

Window analysis defaults to `window_seconds = 1.0` and `hop_seconds = 0.5`. Window and hop options, when exposed, must be finite, positive, bounded, and reflected in JSON so dense transient, clipping, silence, and mix-balance evidence remains reproducible. `loop_boundary_delta` is included when requested or applicable.

Alongside the energy buckets, analysis reports magnitude-spectrum timbre facts measured from native-rate Hann frames: `spectral_centroid_hz` (where the tonal body sits), `spectral_flatness` (≈0 for a pure tone/drone, →1 for rich/noisy content), `air_ratio` (fraction of energy above 4 kHz, i.e. brightness/"air"), `crest_factor` (peak÷RMS), and `strong_peak_count` (prominent spectral peaks; a few evenly spaced peaks signal a single-oscillator voice). These stay non-subjective measurements — they make a flat programmer-art drone or a thin chiptune *visible*, but they do not approve a baseline.

Required live audio warning IDs are:

- `audio_capture.unavailable`
- `audio_capture.failed`
- `audio_capture.duration_too_long`
- `audio_capture.bus_not_found`
- `audio_analysis.near_silent`
- `audio_analysis.clipping_risk`
- `audio_analysis.high_transient_density`
- `audio_analysis.event_spam_risk`
- `audio_analysis.loop_discontinuity_high`
- `audio_analysis.tonal_drone`
- `audio_analysis.dull`
- `audio_analysis.invalid_wav`
- `audio_render.rejected`
- `audio_render.oversized`
- `audio_render.invalid_return`
- `audio_render.failed`

Capture/export failures exit nonzero unless the command is explicitly informational. Failure payloads include the command, target or bus, expected state, actual state, failure kind, and artifact paths when available. `audio_capture.unavailable` and `audio_capture.bus_not_found` are valid structured evidence for optional bus capture support; they are not hidden successes. Analysis warnings do not become pass/fail judgments about whether audio sounds good unless a caller opts into threshold flags or `--strict-warnings`.

### `scenario profile`

One-shot scenario finalization allows a full three-second process-exit grace after a valid current-identity result, including near the original deadline. Completed assertions and clean process completion are reported separately; teardown failure preserves the first assertion failure. Captured stdout/stderr retain the first and last 4 MiB each for inspection, with full file artifacts retained. Truncation adds `output_capture_truncated` and explicit byte counts rather than silently passing omitted output. The nonce-bound `runtime_started` progress event blocks startup retries after scene loading begins; gameplay and teardown failures are never eligible.

`scenario profile <name> --soak <spec.json|yaml>` adds a bounded real-time soak after setup steps and before final assertions. The scenario must declare `time_control: {time_scale: 1.0}` and omit `determinism.fixed_fps`; otherwise the command fails before launch with `soak_realtime_required`. Its declared run timeout should exceed warmup, measurement and teardown (for example, 660 seconds for a 600-second soak). The sampler writes `soak/windows.jsonl` and an updated `soak/soak.json` under the run root after every window; `progress.jsonl` receives a `soak_window` event. A stopped, crashed or timed-out run is incomplete and cannot pass. `data.soak` and the regular profile artifact point to the measured result when the engine returns a result.

Both scenario and live profile accept the same version-1 specification (JSON or YAML):

```yaml
version: 1
warmup_seconds: 0
duration_seconds: 600
window_seconds: 5
monitor_every_frames: 60
state: {provider: qa_soak, epoch: epoch, tick: tick, active: active, phase: phase}
gauges:
  - {name: nodes, path: counts.nodes, max: 500, final_max: 50}
budgets: {frame_p95_ms: 40, frame_max_ms: 150, min_active_ticks_per_second: 55}
```

Defaults are 0-second warmup, 600-second duration, 5-second windows and cached Godot Performance readings every 60 process frames. Warmup plus measured duration is at most 3600 seconds, and duration must divide exactly into 1–60 second windows, at most 720. Each window retains at most 65,536 raw frame intervals in memory; overflow is an explicit failed window. The state source is a registered read-only provider returning a JSON-compatible object. `epoch` distinguishes resets; `tick` is nonnegative and monotonic within one epoch; `active` is boolean. Dot paths select these fields, an optional string phase, and up to 32 uniquely named numeric gauges. Each window records at most 32 named phases with a bounded other bucket, plus first and last phase. Maximum budgets fail only when observed value is strictly greater than the limit; minimum throughput fails only when strictly less. Paused windows retain frame measurements but exclude paused wall time from the active tick-rate denominator. A reset partitions tick deltas and cannot erase previous failures. `final_max` is checked at the end; `max` checks each window's high-water value.

Window records include process, physics and `Engine.get_frames_drawn()` deltas separately. The last is engine-rendered frames, not compositor-presented frames. Frame intervals are observed monotonic gaps between process frames; a gap crossing windows is attributed in full to the first window and skipped windows fail as `soak_no_frames`. At most 120 cached Godot Performance readings are retained per window, with their cadence and omitted count; they are diagnostic, not fresh per-frame timings. A frame gap at least as long as one window fails with `soak_window_overrun`. A declared state source with at least 0.1 active seconds and no ticks fails as `soak_active_stall`, even without a throughput budget. Soak sampling does not force a redraw. The self-describing result retains the normalized spec, measured and warmup durations, every window, first failure, worst windows by metric, provenance (display server, VSync mode, renderer, resolution, time scale, instrumentation and cadence), and a bounded partial window on abnormal termination.

```bash
godot-qa scenario profile <name> --project <project> --json
```

`scenario profile` runs an existing saved scenario once through the same parser, Godot launch path, runtime runner, artifact ownership, display selection, logs, and assertion behavior used by `scenario run`, with profiling collection enabled for that run. It does not introduce scenario syntax and does not change the scenario's gameplay pass/fail contract.

Arguments and options:

- `<name>` is the saved scenario name to resolve.
- `--project <project>` selects the Godot project root through the global option.
- `--headless` and `--windowed` mirror `scenario run` display overrides and remain mutually exclusive.
- `--json` emits the standard JSON result envelope.

Successful JSON output uses `command: "scenario profile"` and includes the normal scenario summary fields such as scenario name, scene, `duration_ms`, `artifacts`, `launch_display`, runtime/process/log summaries, and final outcome. It also includes `data.profile` with:

- `frame_time_ms` and `frame_time_percentiles_ms`: bounded frame timing summary with `sample_count`, `min`, `max`, `p50`, `p90`, `p95`, and `p99`.
- `monitor_counters`: summaries for available Godot `Performance` monitor counters such as process time, physics process time, object/node counts, render timing, frame rate, and static memory usage (`MEMORY_STATIC` and `MEMORY_STATIC_MAX`, useful for spotting leak trends across long soak runs). Optional unavailable monitors are omitted from summaries rather than failing the whole profile.
- `monitor_sample_count`: count of retained runtime monitor samples.
- `step_timings`: per-step timing entries for setup, input, waits, snapshots, assertions, runtime requests, and runner-owned phases when available.
- `step_timing_count`: count of retained timing entries.
- `top_script_methods`: ranked opt-in script spans recorded through `res://addons/godot_qa/qa_profiler.gd`, using the same shape as `session profile`.
- `sample_limits`: configured bounds for retained profile sample history.

`data.artifacts.profile` points to the run-level `profile.json` artifact. Large frame/sample history is written only to that artifact and is bounded by the profiler sample limit; normal `scenario run` output does not include profile sample history. Unwrapped game code returns `top_script_methods.status: "available"` with an empty `methods` array. The separate `profile editor export` command can read the Godot Editor Profiler's Script Functions data when the experimental editor-plugin artifact exists; `scenario profile` itself does not scrape or depend on the editor.

When the profiled scenario declares top-level `time_control`, `scenario profile --json` includes additive `data.time_control` evidence with `original`, `requested`, `effective`, `restored`, `restored_matches_original`, `restoration_status`, and `warnings`. The same evidence is written to `profile.json` so agents can verify that accelerated profiling restored Godot Engine settings after the run. Profiling does not make acceleration a default for later profiles, scenario runs, checks, or live sessions.

Failure categories are reported with the same actionable failure envelope as other agent-facing commands:

- scenario resolution, ambiguity, parse, or contract failures use CLI validation failure types.
- runtime assertion or scenario failures remain scenario failures and do not become successful gameplay validation just because profiling data was collected.
- Godot process and log failures remain process/log failures in the scenario outcome.
- missing or malformed profile payloads fail as profile result/artifact validation problems.
- profile artifact write failures use `type: "artifact_write_failed"` and include the intended `profile.json` path.
- unsupported optional monitor data is represented in the profile payload where available and does not fail the profile when frame and step timing can still be collected.

Profiling is non-privileged by default. The command uses Godot/runtime counters and runner step timing; it does not shell out to Linux `perf`, require root privileges, depend on `/proc/sys/kernel/perf_event_paranoid`, or apply performance thresholds by default.

## Filtered inspection and snapshots

`godot-qa tree` and `godot-qa snapshot` accept the same filter flags for large scene trees:

```bash
godot-qa tree --project examples/sample_project --target qa:start_button --max-depth 1 --json
godot-qa snapshot --name before --target qa:pause_menu --qa-only --metadata-key qa_id --json
```

- `--target <locator>` scopes JSON tree/UI/focus-style capture to the resolved subtree. It uses the shared runtime target resolver documented below.
- `--max-depth <n>` limits descendants below the capture root. `0` returns only the root summary.
- `--qa-only` keeps QA-bearing nodes plus ancestors/descendants needed to keep the filtered tree readable.
- `--include-metadata` explicitly requests compatible metadata serialization. Current full captures include metadata by default; the flag exists so filtered command invocations can state that contract.
- `--metadata-key <key>` and `--metadata-keys <key[,key...]>` serialize only safe allowlisted metadata keys. The forms can be repeated and combined. Safe keys use only ASCII letters, numbers, `.`, `_`, `-`, and `:`.

Invalid filter arguments fail before launching or writing misleading artifacts when the CLI can detect them. Empty `--target`, negative `--max-depth`, and unsafe metadata keys return non-zero JSON failures with `type: "invalid_params"`, the command name, expected value, and actual value. Runtime target resolution failures preserve the resolver failure type such as `target_not_found` or `ambiguous_target`.

Filtering applies only to JSON runtime evidence. Screenshots are never cropped or filtered by these options.

## QA state schema validation

`qa-state validate` validates JSON-compatible project-owned QA state against an explicit schema. It is for stable review playbooks around registered state providers, aggregate state methods, and saved artifacts; it does not require every project, target, provider, or QA method to publish a schema.

Live validation reads state from the running project through the authenticated runtime bridge:

```bash
godot-qa qa-state validate \
  --project examples/sample_project \
  --provider schema_demo \
  --schema examples/sample_project/qa/schemas/schema_demo_state.schema.json \
  --json
```

Legacy method validation reads a QA state method from a target:

```bash
godot-qa qa-state validate \
  --project examples/sample_project \
  --target qa:schema_state_provider \
  --method get_qa_schema_demo_state \
  --schema examples/sample_project/qa/schemas/schema_demo_state.schema.json \
  --json
```

Saved JSON validation does not launch Godot:

```bash
godot-qa qa-state validate \
  --input examples/sample_project/qa/fixtures/schema_demo_state_valid.json \
  --schema examples/sample_project/qa/schemas/schema_demo_state.schema.json \
  --json
```

Trace validation selects repeated state values from a saved trace artifact:

```bash
godot-qa qa-state validate \
  --trace examples/sample_project/qa/fixtures/schema_demo_trace.json \
  --state-path samples[*].state \
  --schema examples/sample_project/qa/schemas/schema_demo_state.schema.json \
  --json
```

`--state-path` supports the documented trace selector form used by this command: field traversal plus array wildcard or index segments such as `samples[*].state` and `samples[0].state`. Each selected value is validated independently, and failures report the selected JSON path.

Live validation can read a registered provider by `--provider`, or call only the same approved zero-argument, read-only-by-convention, JSON-compatible QA state method patterns used by `assert.state_matches`: `qa_*`, `get_qa_*`, and `get_*_qa_state`. Rejected live method requests fail nonzero and identify the method name, target, allowed patterns, and rejection reason such as `name`, `missing_method`, `arguments`, `runtime_error`, or `non_json_return`.

The schema validator intentionally implements a small documented JSON Schema subset:

```text
type
properties
required
items
enum
const
minimum
maximum
minLength
maxLength
minItems
maxItems
additionalProperties
```

Annotation keywords such as `$schema`, `$id`, `title`, `description`, `default`, and `examples` are accepted as documentation. Unsupported validation keywords, including `pattern`, `oneOf`, and `anyOf`, are not silently ignored; they fail with `type: "qa_state_schema_unsupported_keyword"`, the unsupported keyword in `actual`, the supported keyword list in `expected`, and the schema path in `path`.

Passing JSON uses the normal `CommandResult` envelope:

```json
{
  "ok": true,
  "command": "qa_state.validate",
  "message": "validated 1 QA state value(s)",
  "data": {
    "target": "qa:schema_state_provider",
    "method": "get_qa_schema_demo_state",
    "schema": "examples/sample_project/qa/schemas/schema_demo_state.schema.json",
    "summary": {
      "checked_paths": 12,
      "selected_values": 1,
      "warnings": []
    }
  },
  "failures": []
}
```

Schema mismatches exit nonzero and keep actionable errors both in `data.errors` and the first failure's `errors` details:

```json
{
  "ok": false,
  "command": "qa_state.validate",
  "data": {
    "errors": [
      {
        "json_path": "$.rafts[0].velocity.x",
        "schema_path": "$.properties.rafts.items.properties.velocity.properties.x.type",
        "expected": "number",
        "actual": "string",
        "actual_value_preview": "fast"
      }
    ]
  },
  "failures": [
    {
      "type": "qa_state_schema_validation_failed",
      "command": "qa_state.validate",
      "expected": "selected values match schema",
      "actual": "schema validation errors"
    }
  ]
}
```

## Target locators

Commands that resolve runtime targets share the Godot runtime resolver. This includes `node inspect`, `input click`, saved-scenario `click` steps, live and snapshot-backed `assert` commands, visual assertions, and setup commands that accept target locators.

- `qa:<id>` uses canonical `qa.id.<id>` groups as the efficient fast path, then compatibility `qa.<id>` groups, then fallback scans for `qa_id` metadata and `qa_aliases` metadata entries containing either raw ids or prefixed `qa:` locators.
- Metadata-only projects and alias-only migration projects remain supported through fallback scanning.
- Duplicate matches for the same node through group, metadata, and alias sources are deduplicated and count once.
- `qa:`, `path:`, and `name:` locators must resolve uniquely. Ambiguous matches fail with `type: "ambiguous_target"` and include deduplicated matching `path:/root/...` nodes in `actual`.
- Missing `qa:` targets fail with stable `type: "target_not_found"`; payloads may include `candidate_status` and `rejected_candidates` diagnostics for stale, freed, outside-tree, or mismatched fast-path group candidates.

## Temporal assertion commands

Temporal assertion aliases are additive over the shared runtime assertion implementation.

Assertion examples should target durable `qa:` anchors for stable scene systems and visible controls. For fast-changing aggregate gameplay facts, prefer project-owned `get_*_qa_state()` or `get_qa_*_state()` methods on durable anchors, or named `qa_*_probe()` methods for bounded probes, over timed metadata mirroring or debug-label parsing; use screenshot-backed assertions when the contract is rendered perception. Existing examples may prefer project-owned `get_qa_*_state()` methods or named `qa_*_probe()` methods over timed metadata mirroring when that naming already fits the project. Debug labels are diagnostics unless their text is the actual user-facing contract.

Example one-shot command:

```bash
godot-qa assert max-delta-position qa:demo_actor --project examples/sample_project --window-frames 12 --max 32 --json
```

Related temporal aliases use the same bounded-window and sampling contract:

- `godot-qa assert follows ...`
- `godot-qa assert never ...`
- `godot-qa assert within-limits ...`
- saved scenarios use `observe_until` assertions; the runtime bridge command is `assert.observe_until`

Temporal options:

- `--window-frames <positive-int>` or `--window-seconds <positive-number>` (required, exactly one)
- `--sample-every-frames <positive-int>` (default: `1`)
- command-specific bounds such as `--max`, `--tolerance`, `--min`, `--forbidden`, and `--property`

Saved scenario behavior for `observe_until`:

- `godot-qa scenario run <name> --project <project> --json` executes `observe_until` through the shared runtime assertion layer and reports pass/fail in the normal scenario result envelope.
- `observe_until` passes when a property, effective visibility state, or QA method state path becomes true within the configured `window_frames` or `window_seconds`.
- Scenario-level `determinism.temporal_sampling` applies to temporal assertions and supported temporal waits for that scenario run; assertion-local `temporal_sampling` overrides it for that assertion.
- Supported deterministic sampling modes are `fixed_delta`/`fixed_delta_seconds` and `max_frame_duration`/`max_frame_duration_seconds`. They are scoped to the QA run and do not change production gameplay timers outside scenario execution.
- Failures include `condition`, `window`, `sample_every_frames`, `observed_samples`, `first_late_sample` when available, `why_no_match`, and `artifacts.temporal_evidence`.
- S33 CLI aliases and exit-code semantics are unchanged; observe-until validation or assertion failures exit `1`, and CLI parse errors exit `2`.

One-shot generated runtime behavior:

- The CLI generates one runtime temporal assertion request and executes it against the same runtime assertion layer used by scenario assertions.
- The command exits after the bounded window is evaluated.

`--live` behavior:

- With `--live`, the same temporal assertion request is sent to the active session bridge.
- Runtime behavior and failure payload semantics are shared with one-shot mode; only transport/session ownership differs.

Exit codes:

- `0` success (assertion satisfied)
- `1` validation/runtime/assertion failure (including temporal bound violations)
- `2` CLI usage/parse failure

Structured failure payload fields for temporal assertion failures:

- `type`
- `command`
- `target` (or `follower`/`leader` for follow assertions)
- `window`
- `expected`
- `actual`
- `sample`
- `artifacts`

## Proximity assertion commands (S34 additive contract)

Proximity assertions are additive over the shared runtime assertion implementation and are available for saved scenarios and live assertions.

Example one-shot commands:

```bash
godot-qa assert distance qa:actor qa:target --project examples/sample_project --max 96 --json
godot-qa assert distance qa:actor qa:target --project examples/sample_project --min 96 --json
```

Threshold semantics:

- `--max <radius>` asserts distance is within radius (`distance <= max`).
- `--min <radius>` asserts distance is outside radius (`distance >= min`).
- `--max` and `--min` may be combined to assert a bounded distance range (`min <= distance <= max`).
- When both bounds are supplied, `min` must be less than or equal to `max`.
- Negative, `NaN`, or infinite thresholds fail validation with actionable JSON failures.

One-shot generated runtime behavior:

- The CLI generates one runtime proximity assertion request (`assert.distance`) and executes it against the same runtime assertion layer used by scenario assertions.
- The command exits after one sampled assertion result.

`--live` behavior:

- With `--live`, the same proximity assertion request is sent to the active session bridge.
- Runtime behavior and failure payload semantics are shared with one-shot mode; only transport/session ownership differs.

Exit codes:

- `0` success (assertion satisfied)
- `1` validation/runtime/assertion failure (including threshold mismatches and `unsupported_coordinate_space`)
- `2` CLI usage/parse failure

Structured failure payload fields for proximity assertion failures:

- `type`
- `command`
- `a`
- `b`
- `a_resolved`
- `b_resolved`
- `a_position`
- `b_position`
- `expected`
- `actual`
- `coordinate_space`
- `distance`
- `threshold`
- `mismatch_type`
- `artifacts`

## Surface probe assertion commands (S36 additive contract)

Surface probe assertions are additive over the shared runtime assertion implementation.
They are opt-in and project-owned: runtime behavior comes only from explicit QA probe providers and declared surfaces, never from inferred physics layer names, collision shape names, or node names.

Example one-shot commands:

```bash
godot-qa assert surface-probe qa:demo_actor --point 120,420:walkable --point 184,420:blocked --provider qa:demo_actor --provider-method qa_probe_surface --coordinate-space world_2d --project examples/sample_project --json
godot-qa assert surface-matches qa:demo_actor --surface walkable --snapshot after_move --project examples/sample_project --json
godot-qa assert surface-matches qa:demo_actor --surface walkable --window-frames 12 --sample-every-frames 2 --dwell-frames 4 --project examples/sample_project --json
```

Contracted options:

- positional `target` is required for both commands
- `--point <x,y[,z][:expected]>` probe point for `surface-probe`; may be repeated up to runtime bounded limits
- `--provider <locator>` required explicit provider target for `surface-probe`; optional override for `surface-matches` when the actor metadata does not provide the probe target
- `--provider-method <qa_method>` explicit project-owned QA method name; must be QA-prefixed; may also come from target metadata (`qa_surface_probe_method`) at runtime
- `--coordinate-space <world_2d|viewport_2d|world_3d>` explicit sample coordinate space
- `--surface <label>` expected actor/world surface label for `surface-matches`
- `--window-frames`/`--window-seconds`, `--sample-every-frames`, and `--dwell-frames`/`--dwell-seconds` opt `surface-matches` into bounded temporal sampling
- `--snapshot <name>` optional saved snapshot evidence anchor

Bounded/read-only rules:

- Probe requests are bounded by runtime point-count and radius limits.
- Providers are read-only by convention and must not mutate gameplay state.
- Provider responses must be JSON-compatible.

Exit codes:

- `0` success (assertion satisfied)
- `1` validation/runtime/assertion failure
- `2` CLI usage/parse failure

Structured failure payload fields for surface probe assertion failures:

- `type`
- `command`
- `target`
- `provider`
- `provider_method`
- `coordinate_space`
- `points`
- `expected`
- `actual`
- `surface`
- `evidence`
- `artifacts`

## Surface audit command (S69 provider-driven contract)

`surface audit` is a generic bounded audit over a project-provided surface probe. `--setup-script <path>` may be supplied to prepare a deterministic live session before the probe runs. The project owns all gameplay semantics through the provider response; the CLI does not infer meaning from labels, node names, colors, textures, sprites, collision layers, or navigation resources.

Spawn/local region audit:

```bash
godot-qa surface audit qa:surface_provider --project examples/sample_project --center qa:clear_spawn --region-size 320x240 --sample-step 16 --actor-radius 24 --min-spawn-clearance 72 --out .qa/artifacts/manual/surface_audit.json --json
```

Route/corridor audit with heatmap evidence:

```bash
godot-qa surface audit qa:surface_provider --project examples/sample_project --route qa:route_start,qa:route_mid,qa:route_finish --actor-radius 24 --min-corridor-width 72 --sample-step 16 --heatmap-out .qa/artifacts/manual/surface_corridor_heatmap.png --json
```

Mirrored region comparison:

```bash
godot-qa surface audit qa:surface_provider --project examples/sample_project --left-region qa:left_lane --right-region qa:right_lane --mirror-axis qa:centerline --actor-radius 24 --sample-step 16 --json
```

Contracted options:

- positional `provider` is required and resolves to an explicit project QA surface provider such as `qa:surface_provider` or `qa:world`
- exactly one bounded mode is required: `--center` plus `--region-size`, `--route`, or `--left-region` plus `--right-region` plus `--mirror-axis`
- `--sample-step` and `--actor-radius` are required positive finite numbers
- `--min-spawn-clearance`, `--min-corridor-width`, and `--strict-warnings` control threshold warning promotion
- `--out` writes the JSON report, defaulting to a controlled `.qa/artifacts/.../surface_audit.json` path when omitted
- `--heatmap-out <path.png>` writes an optional artifact image; it must be a PNG path and is not a runtime overlay
- `--provider-method` must be one of `qa_surface_probe`, `get_qa_surface_state`, `get_surface_qa_state`, or `qa_probe_surface`

Command JSON success shape:

```json
{
  "ok": true,
  "command": "surface.audit",
  "data": {
    "command": "surface.audit",
    "provider": "qa:surface_provider",
    "provider_method": "qa_surface_probe",
    "metrics": {
      "sample_count": 300,
      "walkable_count": 240,
      "blocked_count": 40,
      "soft_count": 12,
      "unknown_count": 8,
      "spawn_clearance_min_distance": 96.0,
      "spawn_clearance_pass": true,
      "minimum_corridor_width": 112.0,
      "corridor_width_pass": true,
      "hard_block_cluster_count": 2,
      "soft_block_cluster_count": 1,
      "dead_end_pocket_count": 0,
      "largest_dead_end_area": 0.0,
      "route_clearance_fail_points": [],
      "symmetry_delta_score": 0.0,
      "left_right_passability_delta": 0.0,
      "provider_missing_or_invalid": false
    },
    "warnings": [],
    "artifacts": {
      "surface_audit": "/abs/path/.qa/artifacts/manual/surface_audit.json",
      "heatmap": "/abs/path/.qa/artifacts/manual/surface_corridor_heatmap.png"
    }
  },
  "failures": []
}
```

Warning IDs:

- `surface_audit.provider_missing`
- `surface_audit.provider_invalid`
- `surface_audit.unknown_surface_label`
- `surface_audit.sample_limit_exceeded`
- `surface_audit.spawn_clearance_failed`
- `surface_audit.corridor_width_failed`
- `surface_audit.dead_end_pocket_detected`
- `surface_audit.hard_block_cluster_blocks_route`
- `surface_audit.soft_blocker_turns_corridor_risky`
- `surface_audit.symmetry_delta_high`
- `surface_audit.not_enough_data`

By default, the command reports facts and warnings. Explicit threshold flags make the relevant threshold checks meaningful, and `--strict-warnings` promotes threshold warnings such as `surface_audit.spawn_clearance_failed`, `surface_audit.corridor_width_failed`, and `surface_audit.symmetry_delta_high` to command failures with actionable artifacts.

## Skill installation

Current shipped bundled-skill install contract:

```bash
godot-qa install-skills --project /path/to/project --json
```

`install-skills` is a native Rust filesystem operation. It works after moving the binary away from the build checkout. `data.repo_root` reports the runtime distribution or development root; each installed entry uses `source: "embedded"`, `source_path: null`, and `method: "copy"`. It copies the bundled skill embedded in the executable (maintained in `skills/godot-qa/`) into the selected project scope at `.agents/skills/godot-qa`. It does not shell out to `npx skills add`, npm, or any other external package manager.

v1 scope:

- installs one bundled skill: `godot-qa`
- installs to project scope only
- copies files, never symlinks
- reruns replace only `.agents/skills/godot-qa`
- works from a standalone release executable without a source checkout

Success shape:

```json
{
  "ok": true,
  "command": "install-skills",
  "message": "bundled skills installed",
  "data": {
    "project_root": "/path/to/project",
    "repo_root": "/path/to/godot-qa",
    "scope": "project",
    "agent": "codex",
    "installed": [
      {
        "name": "godot-qa",
        "source_path": null,
        "source": "embedded",
        "target_path": "/path/to/project/.agents/skills/godot-qa",
        "method": "copy"
      }
    ]
  },
  "failures": []
}
```

Structured failure types:

- `missing_project_root`
- `skill_target_conflict`
- `skill_install_failed`

Recorder review remains manual: run `scenario record`, inspect the generated artifacts, copy accepted `review.suggested_assertions` into `assert`, and rerun `scenario run`. There is no `scenario approve` command in the current public contract.

## Artifact inspection

Generic artifact summary contract:

```bash
godot-qa artifact summarize <run-root> [--type <type>] [--warnings-only] [--expect <type>] [--markdown-out <path>] [--no-write] --json
```

`artifact summarize` is the agent-facing review index for scenario run roots, live-session roots, bundle roots, and manual artifact folders under `.qa/artifacts/`. Run it before manually opening evidence folders so screenshots, traces, physics reports, audio analysis, event traces, layout audits, logs, errors, profiles, visual audits, and unknown files are indexed in one machine-readable result.

`--type <type>` may be repeated and filters the returned artifact category arrays. Accepted type names are normalized to the public summary categories, including singular aliases such as `screenshot`, `audio_analysis`, `trace`, and `unknown_artifact`.
`--warnings-only` returns only entries that contain warnings or failures.
`--expect <type>` may be repeated; missing expected categories add `artifact_summary.missing_expected_artifact` to `important_findings`.
By default the command writes `<run-root>/artifact_summary.json`. `--no-write` suppresses that JSON artifact.
`--markdown-out <path>` writes a compact Markdown review report that uses relative artifact paths and does not embed base64 screenshots, audio, traces, or other binary payloads.

Success shape:

```json
{
  "ok": true,
  "command": "artifact.summarize",
  "message": "artifact summary completed",
  "data": {
    "run_root": "/path/to/project/.qa/artifacts/live/session-id",
    "summary_path": "/path/to/project/.qa/artifacts/live/session-id/artifact_summary.json",
    "markdown_path": "/path/to/project/.qa/artifacts/live/session-id/review_summary.md",
    "counts": {
      "screenshots": 8,
      "traces": 2,
      "physics_inspections": 1,
      "audio_artifacts": 1,
      "event_traces": 1,
      "errors": 0,
      "warnings": 3
    },
    "important_findings": [
      {
        "source": "audio/current.analysis.json",
        "rule_id": "audio_analysis.clipping_risk",
        "severity": "warning"
      }
    ],
    "summary": {
      "run_root": "/path/to/project/.qa/artifacts/live/session-id",
      "summary_path": "/path/to/project/.qa/artifacts/live/session-id/artifact_summary.json",
      "counts": {},
      "important_findings": [],
      "scenario_results": [],
      "snapshots": [],
      "screenshots": [],
      "screenshot_sequences": [],
      "contact_sheets": [],
      "screenshot_diffs": [],
      "layout_audits": [],
      "traces": [],
      "physics_inspections": [],
      "physics_overlays": [],
      "collision_reports": [],
      "surface_audits": [],
      "audio_artifacts": [],
      "audio_analyses": [],
      "event_traces": [],
      "visual_audits": [],
      "profiles": [],
      "logs": [],
      "errors": [],
      "warnings": [],
      "unknown_artifacts": []
    }
  },
  "failures": []
}
```

If `<run-root>` does not exist or is not a directory, the command exits non-zero with `failure.kind: "artifact_summary.run_root_missing"` and includes the command, target path, expected state, and actual state. If the default `artifact_summary.json` or requested Markdown report cannot be written, the command exits non-zero with `artifact_summary.write_failed` or `artifact_summary.markdown_write_failed` and identifies the output path that failed.

Current saved-snapshot artifact query contract:

```bash
godot-qa artifact query <run-root> --snapshot <name> [--target <target> | --qa-id <id>] [--type <node-type>] [--visible] --json
```

`<run-root>` is a concrete scenario run artifact directory, such as `.qa/artifacts/gameplay_flow/100Z`.
`--snapshot` selects the saved snapshot directory under that run root.
Targets use the same stable prefixes as snapshot assertions:

- `qa:<id>`
- `path:<node-path>`
- `name:<node-name>`

`--qa-id <id>` is shorthand for `--target qa:<id>`.
`--type` filters by Godot node type/class string.
`--visible` keeps only nodes whose snapshot tree entry has `visible: true`.

`artifact query` works best against full `tree.json` artifacts. When `tree.json.filter` is present, a successful `count: 0` can mean either "not present in the scene" or "omitted by the snapshot filter"; snapshot-backed assertions that need omitted evidence report filtered-artifact limits with `snapshot_target_omitted` where possible.

Success shape:

```json
{
  "ok": true,
  "command": "artifact query",
  "message": "artifact query completed",
  "data": {
    "run_root": "/path/to/project/.qa/artifacts/gameplay_flow/100Z",
    "snapshot": "impact",
    "count": 1,
    "matches": [
      {
        "snapshot": "impact",
        "path": "/root/Main/Projectile",
        "name": "Projectile",
        "type": "Area2D",
        "qa_id": "weapon_projectile",
        "visible": true,
        "inside_tree": true,
        "groups": []
      }
    ],
    "artifacts": {
      "tree": "/path/to/project/.qa/artifacts/gameplay_flow/100Z/snapshots/impact/tree.json"
    }
  },
  "failures": []
}
```

Missing or invalid tree artifacts fail with `artifact_read_failed`.
No matches is a successful query with `count: 0`.

Snapshot-backed `assert` also supports:

```bash
godot-qa assert snapshot_exists qa:weapon_projectile --scenario gameplay_flow --run-id 100Z --snapshot impact --json
godot-qa assert snapshot_visible qa:weapon_projectile --scenario gameplay_flow --run-id 100Z --snapshot impact --json
godot-qa assert snapshot_contains_type Area2D --scenario gameplay_flow --run-id 100Z --snapshot impact --json
```

These assertions use the same artifact query engine. `snapshot_contains_type` requires a visible node of that type.

### artifact begin-run / validate-result

For a custom child launched inside an external display wrapper, create a fresh receipt directory before launch and validate it after the wrapper exits:

```text
godot-qa artifact begin-run --out <new-run-root> --json
godot-qa artifact validate-result <run-root> --wrapper-exit <code> [--wrapper-timed-out] --json
```

`begin-run --out` exclusively creates `<new-run-root>` and writes `run.json` with `schema_version: 1`, `kind: "external_child"`, an independently generated `run-` plus 128-bit hex `run_id`, a separate random 128-bit hex `nonce`, and `started_at_unix_ms`. It returns those fields and the expected `result.json` and `child-status.json` paths in the usual `CommandResult` envelope. The historical positional `<new-run-root>` form remains accepted, but exactly one of it and `--out` is required. A repeated directory is an error; an earlier pass cannot be reused. The CLI does not launch the custom child or the wrapper. The run ID is independent of the directory name, so moving the receipt directory preserves its identity.

The custom child writes `result.json` with the same `schema_version`, `run_id`, `nonce`, and `started_at_unix_ms`, plus `completed_at_unix_ms`, boolean `ok`, and a `failures` array. Failure objects use the normal `type`, `message`, `expected`, `actual`, and `artifacts` fields; original failure details remain in the final verdict. An adapter inside the external wrapper must wait for the actual child and atomically write `child-status.json` with the same identity and timestamps, `termination` (`exited`, `signaled`, `timed_out`, `launch_failed`, or `unknown`), and `exit_code` for `exited`. The [Linux shell adapter](../examples/verdict_child_status.sh) is an example for ordinary numeric exit statuses. It expects the root, ID, nonce, and start timestamp from `begin-run` under the `GODOT_QA_RUN_*` environment variables shown in that file. Shell wait status alone cannot prove whether status `128+N` was a signal, so that adapter reports `unknown` for such statuses. An orchestrator with a reliable waitpid receipt may use `signaled`. The orchestrator that enforces an outer deadline passes `--wrapper-timed-out`; absence of output alone is not proof of timeout or crash.

`validate-result` checks both receipts against `run.json` and returns nonzero for stale identity, malformed/missing/incomplete output, assertion/parser/startup failure, child nonzero/crash/timeout, or wrapper failure even if Gamescope returns zero. It writes a separate `verdict.json` without overwriting the child's `result.json`; it does not overwrite an existing verdict. `data.assertions_completed` records whether a valid result was produced, while `data.assertions_passed` records its `ok` field. `data.child_completion`, `data.wrapper`, and `data.first_failure` keep process completion and the first failure distinct. A missing child receipt is always failure even when the result says `ok: true`. Existing saved scenario results remain compatible and continue through the normal scenario finalizer.

When a matching child receipt records nonzero exit before `result.json` exists, a `logs.txt` inside that run root with a Godot `SCRIPT ERROR: Parse Error` or `Parse Error:` line classifies `child_parser_failed`. Otherwise the outcome stays `child_exit_nonzero` unless the inner receipt explicitly reports `failure_phase: "startup"`. The validator never infers crash or timeout from missing output.

### artifact export

`godot-qa artifact export <selection.json> --out <new-directory> --json` builds a portable evidence packet from explicitly selected run roots. The selection file may use relative `run_root` paths resolved beside the selection file; `--out` must not exist. The command never launches Godot, changes source artifacts, or approves screenshot baselines. See [the selection example](../examples/evidence-selection.json).

Selection schema version 1 requires a nonempty `sources` array and a short `title`. Each source needs an ASCII slug `id`, `run_root`, and `result` equal to `result.json` or `verdict.json`. An optional `run_id` is checked against the source result when present. Optional `revision` accepts only `tool`, `project`, and `build` string values; `environment` accepts `renderer`, `display_backend`, `vsync`, `pacing`, boolean `instrumented`, and two positive `resolution` pixel dimensions. Omit unknown metadata rather than copying arbitrary fields.

`images[]` selects `id`, relative PNG `path`, `role: "original"` or `"contact_sheet"`, and optional `snapshot`. Contact sheets are explicitly labeled as derived images (`capture_status: "derived"`); original captures say `"captured"`. A selected PNG must decode within the review size limit. Every selected PNG is copied byte for byte and its source and exported SHA-256 must agree. Each image defaults to `visual_review.status: "unreviewed"`. To record a review, supply `review` with `status: "reviewed"`, `reviewer`, `method`, `scale`, `scope`, `reviewed_at`, and `sha256` of the exact source PNG. A hash mismatch fails export before the destination is created. This is a declaration of an actual human review, not an automatic approval.

`metrics[]` selects a relative JSON artifact `path`, `id`, and a nonempty `fields` map from output field names to JSON pointers. Only numeric, boolean, and null values may be projected; sensitive field names and pointer segments such as tokens, connection PINs, credentials, and secrets are rejected. The export stores both the original source hash and the new summary hash, with `transformation: "scalar_projection_v1"`; it never represents a transformed JSON summary as byte-identical to its source. The same source/export hash distinction applies to every `result-summary.json`, whose transformation is `allowlisted_result_v1`.

The output contains `evidence.json`, relative-link `index.html`, and `sources/<id>/` with selected PNGs and projected JSON summaries. Result summaries retain each declared source's automatic pass/fail/incomplete status and all failure types, including expected-negative runs and failures whose screenshots were captured. A failure entry includes its source JSON pointer (`/failures/<index>`) and the source result SHA-256 appears alongside the summary entry. A contradictory result with `ok: true` and failures is reported as failed; a result missing its `failures` array is incomplete. Failure messages, arbitrary `data`, request/session/receipt files, logs, credentials, and raw JSON are excluded. An absent or malformed result remains visible as an incomplete source and makes the command return nonzero after writing the packet. Relative paths are confined to the selected run root; traversal and symlinks escaping it are rejected. Move the entire output directory together to retain its links.

### artifact prune

Current on-demand retention contract:

```bash
godot-qa artifact prune [--keep-runs <n>] [--dry-run] --json
```

`artifact prune` enforces retention across the whole artifacts root instead of only as a side effect of running something. `scenario run` and `scenario bundle run` prune only what they touched, so a project that enables `artifacts.keep_runs` late — or that ran for months without it — keeps history no command will ever revisit. This sweeps every scenario directory under `.qa/artifacts/`, every `bundles/<name>/`, and the abandoned pooled sessions under `.qa/artifacts/live/` in one pass, applying exactly the rules described under `artifacts.keep_runs` above: run directories are selected by numeric run id, anything that is not an epoch-shaped run id is never a candidate, and `live/latest`, the session it points at, and any session whose Godot process is still running are never removed.

The command resolves the project through the global `--project`, like every other command.

`--keep-runs <n>` sets how many run directories to keep per scenario and per bundle, overriding `artifacts.keep_runs` for this invocation. It must be a positive integer; `--keep-runs 0` is rejected with `type: "invalid_arguments"` and exit code 2, the same refusal the config parser applies to `artifacts.keep_runs: 0`. With neither the flag nor `artifacts.keep_runs` there is no retention policy to enforce, and the command exits 2 with `type: "retention_not_configured"` rather than guessing a limit.

`--dry-run` computes the identical plan, deletes nothing, and reports it under `data.retention.would_remove` and `data.retention.would_remove_paths[]`.

The pooled-session cutoff is taken across all bundles at once: a session is removed only when it started more than one hour before the oldest bundle run that survives this prune, anywhere under `bundles/`. With no bundle runs on disk there is no cutoff that can be justified, so no session is removed.

Success shape:

```json
{
  "ok": true,
  "command": "artifact prune",
  "message": "artifact prune completed",
  "data": {
    "project_root": "/path/to/project",
    "artifacts_dir": "/path/to/project/.qa/artifacts",
    "retention": {
      "keep_runs": 5,
      "dry_run": false,
      "pruned": {
        "scenario_runs": 12,
        "bundle_runs": 3,
        "sessions": 2
      },
      "removed_paths": [
        "/path/to/project/.qa/artifacts/main_menu/1788493321Z"
      ],
      "bytes_reclaimed": 978623488,
      "errors": []
    }
  },
  "failures": []
}
```

`data.retention` always carries `keep_runs`, `dry_run`, `bytes_reclaimed` (a best-effort recursive byte total of the directories removed, or of those that would be removed), `errors[]` as `{path, error}` objects, and one counts object with one path list: `pruned` and `removed_paths[]` for a real prune, `would_remove` and `would_remove_paths[]` for `--dry-run`. The path list is exhaustive so an agent can audit exactly what went away.

Exit 0 means everything asked for succeeded, including having nothing to do. A directory that could not be deleted adds an entry to `data.retention.errors[]` and an `artifact_prune_failed` failure naming the path and the OS error, and the command exits 1 with `ok: false`; the directories that were removed still count. An artifacts root that exists but cannot be listed is also `artifact_prune_failed` with exit 1. An artifacts root that does not exist at all is not an error: a project that has never produced artifacts has nothing to prune and exits 0.

Human output prints one line per category plus the byte total before the usual status line.

## Project validation

Current shipped validation contract through `S24`:

```bash
godot-qa --version
godot-qa doctor
godot-qa doctor --strict-binary-surface
godot-qa doctor --strict-addon-version
godot-qa import
godot-qa check
godot-qa check --scene res://path/to/scene.tscn
godot-qa check --all-scenes
godot-qa check --script res://path/to/script.gd
godot-qa check --all-scripts
godot-qa check --script res://path/to/script.gd --timeout-seconds 8
godot-qa check --tests
godot-qa check --tests --test sample_script_test
godot-qa check --tests --tag smoke
godot-qa check --tests --profile smoke
godot-qa check --tests --profile --test sample_script_test
godot-qa check --tests --profile --tag smoke
godot-qa check --tests --timeout-seconds 5
```

This surface is intentionally bounded by default. `doctor` verifies prerequisites, `import` runs Godot asset import, `check` validates the project `run/main_scene`, `check --scene` and `check --all-scenes` validate scene startup/runtime health, `check --script` and `check --all-scripts` validate GDScript load/compile health through Godot, and `check --tests` runs only the explicit manifest-backed scene/script tests declared under `qa/tests/manifest.json`. `check` is not a heuristic or third-party-framework test runner. Scene and script checks fail if Godot output contains script parse or compilation diagnostics such as `Failed to load script`, `Parse error`, `Parser Error`, or `Compilation failed`, even if the runtime otherwise reaches a result payload. Strict script diagnostic failures include a bounded excerpt with nearby nested/preloaded script line and column context when Godot prints it.

`doctor` checks:

- Godot binary is available.
- Godot version can be read.
- when `godot.launch_prefix` is configured, `data.godot.version_probe` records `prefixed_command`, `final_command`, `direct_fallback_used`, and `fallback_reason` so automation can distinguish normal prefixed probes from the direct-probe fallback.
- if version probing fails, the failure stays machine-readable and includes the probe command, binary path, and expected/actual probe state.
- probe commands are quoted as needed in failure payloads so spaced binary paths remain reproducible.
  Probe `actual` values are currently one of a non-zero exit status, `timeout`, `empty_output`, or `os_error`.
- target project root is resolved from `--project` or the current working directory.
- a missing target root returns `type: "missing_project_root"` instead of silently creating artifact directories there, including when `--project` is a nonexistent relative path.
- addon is installed or installable.
- addon version drift: `doctor --json` always reports `data.cli.version` (the running `godot-qa` CLI version, `env!("CARGO_PKG_VERSION")`), `data.addon.version` (the `version=` value parsed from the installed project addon's `addons/godot_qa/plugin.cfg`, or `null` when no addon is installed there), and `data.addon.source_version` (the same value from the repo/source addon). A `plugin.cfg` that exists but carries no parsable `version=` is reported as `"0.0.0"` so drift stays visible. When an installed addon version is present and differs from the CLI version, `doctor` keeps `ok: true` by default and appends a `data.warnings[]` entry with `type: "addon_version_mismatch"`, `severity: "warning"`, `message`, `addon_path`, `addon_version`, `source_version`, `cli_version`, and `remediation`. Use the addon shipped with the same CLI version. A copied or linked addon can drift from an older installed binary; run `doctor` after updating either component.
- `doctor --strict-addon-version` promotes an installed-addon-vs-CLI version mismatch to an `addon_version_mismatch` failure (with `command`, `path`, `expected` = CLI version, `actual` = installed addon version) for CI or agent environments. Like `--strict-binary-surface`, it only escalates that one check and does not hide or suppress unrelated failures.
- unsupported config spellings: when a `qa/config.json`, `qa/config.local.json`, `qa/config.yml`, or `qa/config.local.yml` file exists (none of which are read), `doctor` keeps `ok: true` and appends a `data.warnings[]` entry with `type: "unsupported_config_file"`, `severity: "warning"`, `path`, `expected`, and `remediation`. It is a warning, never a failure, and there is no strict flag that promotes it.
- unknown config keys: when `qa/config.yaml` or `qa/config.local.yaml` carries a key `godot-qa` does not recognise, `doctor` keeps `ok: true` and appends a `data.warnings[]` entry with `type: "unknown_config_key"`, `severity: "warning"`, `path`, `key`, `expected`, and `remediation`. It is a warning, never a failure, and there is no strict flag that promotes it.
- `qa/scenarios` is reported but not required for `doctor`, `scenario list`, or `check`; commands that need saved scenario lookup still report `missing_scenarios_dir` when it is absent.
- artifact path is writable.
- multiple executable `godot-qa` candidates on `PATH` are checked with bounded help/introspection-only commands. `doctor` does not inspect shell aliases/functions, execute shell snippets, modify `PATH`, delete binaries, or choose a different binary automatically.
- `doctor --json` always reports `data.binary_surface.current_exe`, `data.binary_surface.first_path_binary`, `data.binary_surface.expected_source`, `data.binary_surface.expected_commands`, `data.binary_surface.candidates[]`, and `data.binary_surface.summary`. `first_path_binary` is the first executable file named `godot-qa` discovered while scanning `PATH`, even when the running `doctor` command was launched through an absolute repo-local path.
- every `data.binary_surface.candidates[]` entry includes `path_order`, `raw_path`, `canonical_path`, `is_current_exe`, `is_first_on_path`, `version`, `command_count`, `missing_commands`, `extra_commands`, `probe_errors`, and `remediation`. Probe metadata includes timeout/error state rather than hanging doctor when a candidate is inaccessible, exits non-zero, returns malformed output, or exceeds the safe probe limit.
- if a PATH candidate is stale, shadowing, malformed, inaccessible, or exposes a different leaf command surface, `doctor` keeps `ok: true` when no hard validation failures exist and reports a structured `data.warnings[]` entry with `type: "multiple_godot_qa_binaries"`, `severity: "warning"`, `message`, `current_exe`, `first_path_binary`, `candidate`, `path_order`, `missing_commands`, `extra_commands`, `version`, `probe_errors`, and `remediation`. Remediation text distinguishes stale `.cargo/bin` installs, release archive/PATH entries, and repo-local builds.
- `doctor --strict-binary-surface` promotes incompatible duplicate command surfaces to a `binary_surface_mismatch` failure for CI or agent environments. Strict mode does not hide unrelated Godot, project, addon, scenario, or artifact-dir failures.
- current `doctor` validation failures also carry per-failure `command`, `expected`, and `actual` fields for missing Godot/project/addon checks and artifact-dir writability failures.

When remediating duplicate binary warnings, agents should inspect `data.binary_surface.first_path_binary` and each warning's `remediation` text, then make an explicit project or environment change outside `doctor`: remove or reinstall a stale Cargo source install under `~/.cargo/bin`, update `PATH` so the intended unpacked release archive directory wins or remove older archive directories from `PATH`, and avoid leaving repo-local `target/debug` or `target/release` binaries earlier on `PATH` than the intended installed binary.

`doctor --json` currently returns:

```json
{
  "ok": true,
  "command": "doctor",
  "message": "environment ready",
  "data": {
    "project_root": "/path/to/project",
    "repo_root": "/path/to/godot-qa",
    "qa_dir": "/path/to/project/qa",
    "scenarios_dir": "/path/to/project/qa/scenarios",
    "godot": {
      "binary": "/path/to/godot",
      "version": "4.5.2.stable.official",
      "version_probe": {
        "prefixed_command": "xvfb-run -a /path/to/godot --version",
        "final_command": "/path/to/godot --version",
        "direct_fallback_used": true,
        "fallback_reason": "prefixed probe exited non-zero"
      }
    },
    "addon": {
      "status": "installed",
      "path": "/path/to/project/addons/godot_qa",
      "source_path": "/path/to/godot-qa/addons/godot_qa",
      "version": "0.1.0",
      "source_version": "0.1.0"
    },
    "cli": {
      "version": "0.1.0"
    },
    "artifact_dir": {
      "path": "/path/to/project/.qa/artifacts",
      "writable": true
    },
    "binary_surface": {
      "current_exe": "/path/to/godot-qa",
      "first_path_binary": "/path/to/godot-qa",
      "expected_source": "rust_public_command_surface",
      "expected_commands": ["doctor", "import", "check"],
      "candidates": [
        {
          "path_order": 0,
          "raw_path": "/path/to/godot-qa",
          "canonical_path": "/path/to/godot-qa",
          "is_current_exe": true,
          "is_first_on_path": true,
          "version": {
            "version": "godot-qa 0.1.0",
            "build": null,
            "status": 0,
            "timed_out": false,
            "error": null
          },
          "command_count": 48,
          "missing_commands": [],
          "extra_commands": [],
          "probe_errors": [],
          "remediation": "release archive or PATH entry matches the expected command surface"
        }
      ],
      "summary": {
        "candidate_count": 1,
        "incompatible_candidate_count": 0,
        "probe_error_count": 0
      }
    },
    "warnings": []
  },
  "failures": []
}
```

With `--verbose`, `doctor` also adds:

```json
{
  "data": {
    "diagnostics": {
      "cwd": "/path/to/current/working/directory",
      "requested": {
        "project": "/path/passed/to/project",
        "artifact_dir": "qa/output",
        "godot_bin": "/path/to/godot"
      },
      "paths": {
        "project_root_exists": true,
        "scenarios_dir_exists": true,
        "project_addon_exists": false,
        "repo_addon_exists": true
      }
    }
  }
}
```

Usage/parse failures return `command: "parse"` with `type: "usage_error"` and exit status `2`.
This includes unknown commands and missing required nested subcommands such as `godot-qa scenario`.

`import` runs Godot asset import and returns the normal `CommandResult` envelope with structured launch/import failures. It intentionally validates only the prerequisites it needs: Godot binary discovery, project-root existence, and writable artifact output. Launch failures and non-zero exits keep artifact paths attached so follow-up automation can inspect logs and result payloads.

Example success shape:

```json
{
  "ok": true,
  "command": "import",
  "message": "project import completed",
  "data": {
    "project_root": "/path/to/project",
    "duration_ms": 1200,
    "artifacts": {
      "root": "/path/to/project/.qa/artifacts/import/20260422T120000Z",
      "meta": "/path/to/project/.qa/artifacts/import/20260422T120000Z/meta.json",
      "result": "/path/to/project/.qa/artifacts/import/20260422T120000Z/result.json",
      "logs": "/path/to/project/.qa/artifacts/import/20260422T120000Z/logs.txt"
    }
  },
  "failures": []
}
```

`check` is a bounded validator plus one explicit opt-in test mode. It:

- runs asset import for scene startup checks;
- instantiates target scenes for `--scene` and `--all-scenes`;
- validates one requested scene or enumerates all project scenes with explicit exclusions;
- loads requested GDScript files for `--script` and `--all-scripts` without instantiating them and without requiring `run_qa_test()`;
- runs explicit manifest-declared `scene` and `script` tests for `--tests`;
- fails on runtime errors.

`check` reuses `doctor`-style project validation prerequisites and also requires the addon to be installed in the selected target project because it launches `res://addons/godot_qa/qa_runner.gd` inside that project.

Scene enumeration rules for `--all-scenes`:

- include `.tscn` and `.scn`
- exclude `res://addons/**`
- exclude `res://.godot/**`
- exclude `res://.qa/**` and legacy `res://qa/artifacts/**`

Script enumeration rules for `--all-scripts`:

- include `.gd`
- exclude `res://addons/**`
- exclude `res://.godot/**`
- exclude `res://.qa/**` and legacy `res://qa/artifacts/**`

`check --script` requires a project-local `res://*.gd` target and defaults to an 8 second owned-process timeout. `--timeout-seconds <positive-number>` may override that bound for `--script` and `--all-scripts`; timeout failures use `type: "check_script_timeout"` with the effective command, target, timeout metadata, cleanup state, and artifacts. This mode is Godot-backed script load/compile validation. It is not a standalone static parser, does not run the project scene, does not instantiate the script, and does not execute a project test method.

`check` and `check --scene` return one bounded scene result under `data`; bare `check` uses `run/main_scene` from `project.godot`. `check --all-scenes` keeps per-scene status under `data.scenes`, `check --script` and `check --all-scripts` keep per-script status under `data.scripts`, and `check --tests` keeps per-test status under `data.tests`. Actionable validation failures are expected to identify the command family, target, expected state, actual state, and artifact paths when available.

A missing or empty `qa/scenarios` directory is a valid startup state for `check`. `check --all-scenes` can still enumerate zero or more project scenes, and `check --tests` reports manifest-specific setup failures such as `test_manifest_missing` rather than stopping at scenario scaffold validation.

`check --tests` requires:

- `qa/tests/manifest.json`
- `version: 1`
- a `tests` array; an empty array is valid and returns a zero-test no-op result
- unique stable `id` values
- `kind` values of `scene` or `script`
- `target` values that stay under the selected project root
- optional entry-level `tags` arrays of unique, non-empty strings
- optional top-level `profiles` object whose names expand to explicit `tests` ids and/or `tags`; empty or currently unmatched profiles are allowed so default `check --tests` remains usable, but selecting one explicitly fails with `empty_selection`
- optional per-test `fixed_fps` values for fastest frame pacing; when omitted, tests derive the value from `project.godot` using the same scenario rule
- optional per-test `time_control` values that use the saved-scenario acceleration schema; invalid values fail before launch with `type: "test_manifest_invalid"`
- optional manifest-level and entry-level `timeout_seconds` values. Entry-level timeouts take precedence over manifest-level timeouts, and CLI `--timeout-seconds <seconds>` takes precedence over both manifest scopes for the selected run.
- optional `slow_summary_threshold_ms` at the manifest or test-entry level. The default is `1000` ms; lower fixture-specific thresholds are useful for deliberate slow-summary tests. Slow summaries are informational and do not fail passing runs.

`check --tests` accepts selection filters:

- `--test <id>` selects one manifest test id and may be repeated.
- `--tag <tag>` selects all tests with a matching manifest tag and may be repeated. Multiple tags are a union, not an intersection.
- `--profile <name>` selects one manifest profile. Profiles may contain `tests`, `tags`, or both; their selected tests are also a union.
- bare `--profile` enables diagnostic profiling for the selected manifest tests. It may be combined with `--test <id>`, `--tag <tag>`, or `--profile <name>`, for example `check --tests --profile --tag smoke` or `check --tests --profile --profile smoke`.
- `--batch` is an additive, explicit opt-in for `check --tests`. It is not the default and trades per-test Godot process isolation for one shared Godot process.
- `--timeout-seconds <positive-number>` sets a run-level timeout for each selected manifest test. Values must be positive finite numbers; zero, negative values, non-numbers, and unsupported duration strings fail before Godot launch with `type: "invalid_params"`.

`--test`, `--tag`, and `--profile` are mutually exclusive selection modes. Ambiguous combinations fail before Godot launch with `type: "test_selection_invalid"`. Unfiltered `check --tests` remains the compatibility default and runs the full manifest.
Batch mode is compatible with the same `--test`, `--tag`, and `--profile` selection filters. Selected manifest entries may opt out with `"batch": false`; if such an entry is selected while `--batch` is present, the command fails before launch with `type: "test_selection_invalid"` and identifies the rejected test id and target instead of falling back to isolated execution.

Example manifest:

```json
{
  "version": 1,
  "tests": [
    {
      "id": "main_menu_startup",
      "kind": "scene",
      "target": "res://ui/main_menu.tscn",
      "startup_frames": 5,
      "tags": ["smoke", "ui", "scenario-adjacent"]
    },
    {
      "id": "sample_script_test",
      "kind": "script",
      "target": "res://qa/tests/script_smoke.gd",
      "tags": ["smoke", "script"]
    }
  ],
  "profiles": {
    "smoke": {
      "tags": ["smoke"]
    },
    "full": {
      "tests": ["main_menu_startup", "sample_script_test"]
    }
  }
}
```

Tags are optional; existing manifest version `1` entries without `tags` remain valid. The reserved tag names are `all`, `none`, and `default`; they are rejected as manifest tags and profile tag selectors so future compatibility aliases can be added without changing existing project meaning.

Selection validation failures are actionable and occur before running Godot:

- duplicate test ids fail with `type: "test_manifest_invalid"`.
- duplicate tags on one test entry fail with `type: "test_manifest_invalid"`.
- empty tag strings, non-string tags, reserved tags, and malformed `profiles` definitions fail with `type: "test_manifest_invalid"`.
- unknown `--test`, `--tag`, or `--profile` filters fail with `type: "test_selection_invalid"` and report the requested filter and known values where available.
- filters that select no tests fail with `type: "test_selection_invalid"`; the CLI must not silently fall back to all tests.

Selection examples:

```bash
godot-qa check --tests --test sample_script_test --project examples/sample_project --json
godot-qa check --tests --tag smoke --project examples/sample_project --json
godot-qa check --tests --profile smoke --project examples/sample_project --json
godot-qa check --tests --profile --test sample_script_test --project examples/sample_project --json
godot-qa check --tests --profile --tag smoke --project examples/sample_project --json
godot-qa check --tests --batch --tag smoke --project examples/sample_project --json
godot-qa check --tests --test slow_manifest_test --timeout-seconds 5 --project examples/sample_project --json
godot-qa check --tests --project examples/sample_project --json
```

In batch mode, successful JSON includes `data.execution_mode: "batch"` and `data.artifacts.batch_summary` pointing to `.qa/artifacts/check/<run-id>/batch/summary.json`. Isolated `check --tests` remains `data.execution_mode: "isolated"` and preserves current per-test result fields.
All `check --tests` runs include additive `data.progress` evidence and `data.artifacts.progress` pointing to `.qa/artifacts/check/<run-id>/progress.jsonl`. Each JSONL event identifies the active test `id`, `kind`, `target`, event state (`started`, `heartbeat`, or `finished`), started time, elapsed time, and relevant artifact paths. Persisted progress evidence applies the same secret redaction rules used by artifact metadata and must not expose session tokens or unredacted secret-looking project args.
All `check --tests` runs include additive `data.slow_summary` evidence and `data.artifacts.slow_summary` pointing to `.qa/artifacts/check/<run-id>/slow-summary.json`. The summary keeps at most the ten selected tests whose completed `duration_ms` is at or above their configured threshold, sorted slowest first. Exceeding the slow threshold never changes `ok`; only validation failures or explicit timeouts do.
When bare `--profile` is present, successful and failed validation JSON also includes additive `data.profile` and `data.artifacts.profile` pointing to `.qa/artifacts/check/<run-id>/profile.json`. `data.profile` is a bounded command-summary object with `frame_time_ms`/`frame_time_percentiles_ms`, `monitor_counters`, `monitor_sample_count`, `unsupported_monitors`, aggregate `top_script_methods`, `sample_limits`, slow selected-test summaries, bounded `per_test_timings`, `per_test_timing_count`, and the profile artifact reference. The profile artifact carries the fuller bounded sample history, aggregate script spans, and selected-test outcome list. Normal unprofiled `check --tests` output remains compatible and does not include `data.profile`, `data.artifacts.profile`, or large profile sample history.
Profile timing rows are runner-owned diagnostic evidence. Runtime rows from Godot use `profile.step_timings`; the CLI also adds a `check_test_total` row per selected test so agents can identify slow manifest entries even when a project script has no method-level instrumentation. Rows may include process startup, scene startup, script load, `run_qa_test()`, teardown, artifact writes, and other runner phases when available. Sample history is bounded by `sample_limits.max_frame_samples`; command JSON keeps only a bounded timing subset, while `profile.json` is the artifact of record for inspection.
Profiling is diagnostic-only and does not add default performance thresholds. A failing manifest test under profiling is still a failed `check` result with normal per-test failures flattened into the aggregate `failures` array. Process failures, timeouts, runtime errors, script errors, assertion failures, malformed profile payloads, and profile artifact write failures remain non-zero failures; malformed profile payloads use `type: "profile_result_invalid"` and artifact write failures use `type: "artifact_write_failed"`. Optional monitor counters that Godot cannot provide are reported in `unsupported_monitors` or omitted from counter summaries and do not fail an otherwise valid profile.
Each selected test still records `id`, `kind`, `target`, `ok`, `duration_ms`, `runtime_errors`, `artifacts`, and `failures`. Per-test failures are flattened into the aggregate `failures` array as in isolated mode; batch mode must not collapse multiple per-test failures into one generic batch failure. Ordinary selected-test failures fail the aggregate command but do not stop the runner from attempting later selected tests, so later-test results remain visible when the Godot process can continue. Infrastructure failures that prevent trustworthy continuation may stop the batch and must identify the active test id, expected state, actual state, and artifact paths.
Batch cleanup diagnostics should report contamination evidence when practical, including held input state, unreleased scene instances, unreleased script instances, modified `Engine` settings, and runtime errors observed between entries. Such diagnostics are attached to the active test result or the batch summary artifact and are also represented in aggregate failures when they affect the command outcome.

Timeout failures are validation/runtime infrastructure failures, not gameplay assertion failures. A manifest test timeout uses `type: "check_test_timeout"` and includes `command: "check"`, the active test id/kind/target, elapsed time, configured timeout, expected state, actual state, owned-process cleanup evidence, and artifact paths including `progress.jsonl`, `slow-summary.json` when written, the check `result.json`, and the active test directory. A scenario timeout uses `type: "scenario_run_timeout"` and includes `command: "scenario run"`, the active step or assertion index when known, operation kind, redacted target/locator context, elapsed time, configured timeout, expected state, actual state, cleanup evidence, and the run artifact paths.

When a scenario declares a run timeout and frame pacing is active (no `time_control`), the engine enforces the timeout as a frame budget (`run.frame_budget` in `request.json`, `frames = ceil(timeout_seconds x fixed_fps)`) and a blown budget fails with `type: "scenario_frame_budget_exceeded"` via a clean engine exit (normal `result.json`, `timed_out: false`) instead of a process kill. Frames are load-independent under `--fixed-fps`, so host load cannot time out a correct scenario. The process kill deadline and the warm transport wall clock become hang backstops only: declared timeout with 3x slack, capped at 1800 seconds, never below the declared value. `scenario_run_timeout` remains the failure for true hangs and for wall-clock-only scenarios (`time_control` or no declared timeout).

Script entries must expose:

```gdscript
func run_qa_test() -> Dictionary:
    return {
        "ok": true,
        "failures": []
    }
```

`run_qa_test()` must return a JSON-compatible dictionary. `ok` must be a boolean, `failures` must be an array of dictionaries, and optional extra machine-readable data may live under `data`.

Example aggregate shape:

```json
{
  "ok": false,
  "command": "check",
  "message": "project check failed",
  "data": {
    "project_root": "/path/to/project",
    "mode": "all_scenes",
    "checked_scene_count": 5,
    "import": {
      "ok": true,
      "artifacts": {
        "logs": "/path/to/project/.qa/artifacts/check/20260422T120000Z/import/logs.txt"
      }
    },
    "artifacts": {
      "root": "/path/to/project/.qa/artifacts/check/20260422T120000Z",
      "meta": "/path/to/project/.qa/artifacts/check/20260422T120000Z/meta.json",
      "result": "/path/to/project/.qa/artifacts/check/20260422T120000Z/result.json"
    },
    "scenes": [
      {
        "scene": "res://ui/main_menu.tscn",
        "ok": true,
        "duration_ms": 87,
        "startup_frames": 5,
        "runtime_errors": [],
        "artifacts": {
          "root": "/path/to/project/.qa/artifacts/check/20260422T120000Z/scenes/ui-main-menu-824709",
          "logs": "/path/to/project/.qa/artifacts/check/20260422T120000Z/scenes/ui-main-menu-824709/logs.txt",
          "result": "/path/to/project/.qa/artifacts/check/20260422T120000Z/scenes/ui-main-menu-824709/result.json"
        },
        "failures": []
      }
    ]
  },
  "failures": [
    {
      "type": "runtime_errors_present",
      "command": "check",
      "target": "res://ui/fixtures/runtime_error_probe.tscn"
    }
  ]
}
```

Example `check --tests` aggregate shape:

```json
{
  "ok": true,
  "command": "check",
  "message": "project check passed",
  "data": {
    "project_root": "/path/to/project",
    "mode": "tests",
    "execution_mode": "isolated",
    "manifest": "/path/to/project/qa/tests/manifest.json",
    "total_test_count": 2,
    "selected_test_count": 1,
    "skipped_test_count": 1,
    "checked_test_count": 2,
    "selection": {
      "mode": "tag",
      "tests": [],
      "tags": ["smoke"],
      "profile": null,
      "selected_ids": ["sample_script_test"],
      "skipped_ids": ["main_menu_startup"]
    },
    "import": {
      "ok": true,
      "artifacts": {
        "logs": "/path/to/project/.qa/artifacts/check/20260422T120000Z/import/logs.txt"
      }
    },
    "artifacts": {
      "root": "/path/to/project/.qa/artifacts/check/20260422T120000Z",
      "meta": "/path/to/project/.qa/artifacts/check/20260422T120000Z/meta.json",
      "result": "/path/to/project/.qa/artifacts/check/20260422T120000Z/result.json",
      "batch_summary": null
    },
    "tests": [
      {
        "id": "sample_script_test",
        "kind": "script",
        "target": "res://qa/tests/script_smoke.gd",
        "ok": true,
        "duration_ms": 4,
        "runtime_errors": [],
        "artifacts": {
          "root": "/path/to/project/.qa/artifacts/check/20260422T120000Z/tests/script-smoke-abc123",
          "request": "/path/to/project/.qa/artifacts/check/20260422T120000Z/tests/script-smoke-abc123/request.json",
          "result": "/path/to/project/.qa/artifacts/check/20260422T120000Z/tests/script-smoke-abc123/result.json",
          "logs": "/path/to/project/.qa/artifacts/check/20260422T120000Z/tests/script-smoke-abc123/logs.txt"
        },
        "failures": []
      }
    ]
  },
  "failures": []
}
```

`checked_test_count` remains for compatibility and equals `selected_test_count` for filtered runs. New automation should prefer `total_test_count`, `selected_test_count`, and `skipped_test_count` because skipped manifest entries are not passed tests. Filtered runs must preserve normal per-test pass/fail behavior for every selected test: a failing selected test makes the command fail, and skipped tests are represented only in selection metadata.

`check` is intentionally not a heuristic or third-party test-framework adapter. `check --tests` only runs the manifest-backed contract above, while scenarios remain the preferred path for interactive flows, screenshots, and richer step-by-step validation.

Manifest-backed tests use default fastest frame pacing unless they declare `time_control`. Each `data.tests[]` entry for a fastest run includes `frame_pacing` metadata with the fixed FPS source. Tests that request `time_control` instead may include additive Godot-owned `time_control` evidence copied from the runtime result. The evidence uses `original`, `requested`, `effective`, `restored`, `restored_matches_original`, `restoration_status`, and `warnings`; warning evidence such as `time_control_physics_pairing_missing` does not replace the normal test pass/fail fields.

Planned post-S24 validation expansion:

- `S25`: opt-in important-HUD layout contracts for non-interactive labels/panels
- `S26`: bounded world/camera inspection plus assertions such as `assert in_camera_view`
- `S27`: runtime-target ergonomics for dynamic nodes and more actionable locator/diagnostic flows

## MCP wrapper

```bash
godot-qa mcp
godot-qa mcp --project /path/to/project --godot-bin /path/to/godot --artifact-dir qa/out --verbose
```

`mcp` starts a stdio MCP server that reuses the shipped CLI behavior instead of introducing a second scenario/session/layout implementation path. The server shells back into the same `godot-qa` binary with `--json` for each tool call.

Notes:

- top-level defaults from `godot-qa mcp --project ... --godot-bin ... --artifact-dir ... --verbose` apply to all MCP tools
- per-tool overrides for `project`, `godot_bin`, `artifact_dir`, and `verbose` win over server-level defaults
- `--json` remains a parsed top-level flag for CLI consistency, but `godot-qa mcp` does not emit normal CLI JSON on stdout because stdout is reserved for MCP protocol traffic
- the shipped `godot-qa mcp` entrypoint does not require a separate package install

Stable MCP tools in S13:

```text
doctor
scenario_list
scenario_run
session_start
session_stop
snapshot
input_tap
input_click
tree_dump
ui_dump
focus_get
layout_audit
layout_audit_current
logs
```

Tool result contract:

- every tool returns the existing `CommandResult` object as MCP `structuredContent`
- failed CLI validations/runtime assertions remain normal tool results with `ok: false`; they do not become MCP transport failures
- a short text summary is also returned for human-oriented MCP clients
- artifact paths remain filesystem paths in the returned payload; S13 does not stream screenshot bytes through MCP

## Review templates

```bash
godot-qa review list-templates --json
godot-qa review init scenario-smoke --project <project> --name smoke_review --scene res://ui/main_menu.tscn --out .qa/review-scenarios/smoke_review.yaml --json
godot-qa review init deterministic-runtime-review --project <project> --name deterministic_review --scene res://ui/main_menu.tscn --project-arg --seed --project-arg 123456 --frames 60 --out .qa/review-scripts/deterministic_review.yaml --json
godot-qa session run-script .qa/review-scripts/deterministic_review.yaml --project <project> --json
godot-qa scenario run --file .qa/review-scenarios/smoke_review.yaml --project <project> --json
```

`review list-templates` returns the built-in project-agnostic review template inventory. In `--json` mode, `data.templates[]` contains `id`, `description`, `output_type`, `generated_command_type`, `required_parameters`, `optional_parameters`, and `required_tools`.

Built-in template IDs:

- `scenario-smoke`: writes a scenario harness for `scenario run --file`.
- `live-visual-review`: writes a live script for visual snapshots and screenshot-sequence evidence.
- `live-physics-review`: writes a live script for target-scoped trace and physics collision-report evidence when targets are supplied.
- `live-audio-review`: writes a live script for bounded audio capture and analysis evidence.
- `live-event-review`: writes a live script that uses caller-supplied `--target` or `--trace-target` plus `--event-provider` for generic event review evidence.
- `visual-physics-audio`: writes one live script that combines screenshot sequence, target-scoped physics evidence, audio capture, logs, and errors.
- `deterministic-runtime-review`: writes a live script that forwards `project_args` exactly and captures bounded snapshots, trace or tree/ui state, logs, and errors over time.

`review init <template>` writes a generated scenario file under the requested `--out` path for scenario templates, or a generated live-script file for live templates. Required parameters are `--name`, `--scene`, and `--out`. Optional parameters are template-dependent and include repeatable `--target`, repeatable `--project-arg`, `--frames`, `--screenshot-every`, `--audio-seconds`, `--trace-target`, and `--event-provider`. `--trace-target` is appended to the same target list as `--target`. `--project-arg` values are stored as a string array and forwarded after Godot's `--` separator by the generated scenario or live `session_start`; the CLI does not interpret values such as `--seed`.

`review init --json` returns:

```json
{
  "ok": true,
  "command": "review init",
  "data": {
    "template": "visual-physics-audio",
    "output_file": ".qa/review-scripts/current_review.yaml",
    "run_command": "godot-qa session run-script .qa/review-scripts/current_review.yaml --project <project> --json",
    "required_tools": ["session run-script", "screenshot sequence", "physics collision-report", "audio capture"],
    "warnings": [],
    "project_args": ["--seed", "123456"]
  },
  "failures": []
}
```

Generated files are not auto-run. The current public CLI contract does not include `review run`; run generated live scripts with `session run-script` and generated scenario harnesses with `scenario run --file`. Template-generated files use the existing documented scenario and live-script operations, and generation must fail rather than invent unsupported YAML operations.

Review templates accelerate exploratory QA without replacing saved scenarios, final human review, or manual screenshot baseline approval. Generated review evidence is normal scenario/live evidence under `.qa/artifacts` or caller-specified review artifact paths; templates do not approve or write `qa/baselines`.

## Scenario commands

```bash
godot-qa scenario list
godot-qa scenario run <name>
godot-qa scenario run --file .qa/review-scenarios/current_review.yaml
godot-qa scenario run <name> --headless
godot-qa scenario run <name> --windowed
godot-qa scenario run <name> --artifact-dir .qa/artifacts/<name>
godot-qa scenario lint --project <project> [--scenario <name>] [--strict] --json
godot-qa scenario init <name> --scene res://ui/menu.tscn --viewport 1280x720
godot-qa scenario init <name> --scene res://ui/menu.tscn --out qa/scenarios/candidates/<name>.json
godot-qa scenario record <name> --scene res://ui/menu.tscn --viewport 1280x720
godot-qa scenario sweep <name> --seeds 1,2,3
godot-qa scenario sweep <name> --seed-start 100 --seed-count 20 --repeat 2 --require-all-pass
```

`scenario approve` is not part of the current public CLI contract. Recorder review is currently manual: inspect the generated artifacts, copy accepted assertions into `assert`, and rerun `scenario run`.

### `scenario sweep`

`scenario sweep` runs ONE scenario across many seeds and aggregates the results — the backbone for difficulty-curve and generator-fuzz gates:

```bash
godot-qa scenario sweep procgen_room_smoke --project . --seeds 1,2,3 --json
godot-qa scenario sweep procgen_room_smoke --project . --seed-start 100 --seed-count 20 --repeat 2 --json
```

- Seed selection uses exactly one mode: an explicit `--seeds 1,2,3` list (duplicates are rejected; rerun a seed deliberately with `--repeat`) or a contiguous `--seed-start N --seed-count K` range. `--repeat K` runs every selected seed K times back to back, and the expanded run count (`seeds x repeat`) is capped at 1000.
- Every seed runs the existing single-scenario cold path with the seed injected: the sweep strips any `--seed` the scenario (or a repeatable `--project-arg` passthrough) already declared, appends `--seed=<s>` as the LAST project arg, and overwrites `determinism.seed` when the scenario declares one. Precedence: the sweep seed always wins over the scenario's own `--seed` project arg and `determinism.seed`.
- Artifacts form ONE aggregate identity with per-seed children: `.qa/artifacts/<scenario>/sweep-<run-id>/seed-<s>/...` holds each child's untouched `scenario run` artifact tree, and the aggregate `sweep.json` at the sweep root records `{scenario, seeds, per_seed: [{seed, ok, failed_asserts, duration_ms, artifact_dir, state_records}], summary: {total, passed, failed, pass_rate}}`. Each `per_seed` entry's `state_records` carries that child run's `record_state` evidence map (null when the seed recorded none) so per-seed balance/outcome values are readable straight from `sweep.json`; aggregation stays agent-side. Single-run artifact behavior is unchanged.
- A failing seed does NOT abort the sweep — collecting the pass/fail distribution is the point. The command exits non-zero only for infrastructure errors (invalid selection, missing scenario, artifact write failures) or when `--require-all-pass` is set and any seed run failed (`type: "sweep_seeds_failed"`).
- `--headless`/`--windowed` apply to every seed run, and `--file <path>` accepts an external harness scenario exactly like `scenario run --file`.
- v1 sweeps are cold-per-seed by design: warm bundle sessions reuse one fixed launch configuration, while a sweep changes launch `project_args` per seed, so a fresh process per seed is the faithful production path.

The human summary line reports the distribution (`scenario sweep completed: 18/20 seed run(s) passed (pass_rate 0.90)`); `--json` mode prints the aggregate (`data.per_seed`, `data.summary`, `data.artifacts.sweep`).

`scenario list --json` returns parsed scenario names where possible, falls back to the file stem otherwise, scans JSON plus YAML (`.yaml`/`.yml`) scenario files, and emits absolute file paths. For S14, the canonical checked-in sample project lives under `examples/sample_project`, so common verification runs should pass `--project examples/sample_project` and expect names such as `sample_main_menu_start` and `sample_settings_layout`:

```bash
godot-qa scenario list --project examples/sample_project --json
godot-qa scenario run sample_main_menu_start --project examples/sample_project --json
godot-qa scenario run sample_setup_steps --project examples/sample_project --json
godot-qa scenario run sample_settings_layout --project examples/sample_project --json
godot-qa layout audit --project examples/sample_project --scene res://ui/settings_menu.tscn --json
```

Example `scenario list` output:

```json
{
  "ok": true,
  "command": "scenario list",
  "message": "found 3 scenario(s)",
  "data": {
    "scenarios": [
      {
        "name": "sample_main_menu_start",
        "path": "/path/to/project/qa/scenarios/examples/sample_main_menu_start.yaml"
      },
      {
        "name": "sample_setup_steps",
        "path": "/path/to/project/qa/scenarios/examples/sample_setup_steps.json"
      }
    ]
  },
  "failures": []
}
```

With `--verbose`, `scenario list` also adds `data.diagnostics` using the same request/path structure plus `scenario_files_scanned`.
If the selected `--project` root does not exist, `scenario list` returns exit status `1` with `command: "scenario list"` plus a `missing_project_root` failure. A missing or empty `qa/scenarios` directory is a valid startup state: `scenario list --json` returns exit status `0`, `ok: true`, and an empty `data.scenarios` array. With `--verbose`, `data.diagnostics.paths.scenarios_dir_exists` preserves whether the directory was actually present.
Failure entries include their own `command`, `expected`, and `actual` fields so hard path failures stay actionable without inspecting the top-level message.
Abstract scenario templates (files with top-level `abstract: true`) are excluded from `scenario list`, so they never appear as runnable scenarios; they are only pulled in through a concrete child's `extends` (see docs/scenario-format.md, "Scenario templates").

`scenario init` emits a valid JSON scenario template without launching Godot or opening a live recorder port. With `--out`, it writes that template to disk, creates parent directories, and refuses to overwrite an existing file with `type: "scenario_output_exists"`.

`scenario run` now implements the one-shot execution path used through S4. It resolves a scenario on the CLI side, writes `meta.json` and `request.json`, launches Godot through `qa_runner.gd`, and returns the runtime `result.json` in the standard command envelope. Runs that save or compare screenshots, or use screenshot-backed visual assertions, require real rendered pixels and launch windowed unless a safely detected virtual display wrapper is selected; non-screenshot runs may launch headless. `--headless` forces the one-shot Godot process to include `--headless`, while `--windowed` forces a windowed process; the flags conflict. Saved scenarios support fixture setup steps (`teleport_node`, `fixture_teleport`, `spawn_scene`, `set_property`, `call_method`) so tests can establish deterministic starting state before exercising player-facing input; `call_method` only invokes methods beginning with `qa_`.

`scenario run --file <path>` executes one explicit JSON/YAML scenario file outside the saved `qa/scenarios` registry. Relative paths resolve from the selected project root, so ignored review harnesses such as `.qa/review-scenarios/current_review.yaml` can be executed without appearing in `scenario list`. The file must use `.json`, `.yaml`, or `.yml`; JSON-shaped content is accepted in YAML files for generated harnesses. Artifact layout and scenario validation match normal `scenario run`.

Saved scenarios may include `godot_args` and `project_args`. Both are arrays of non-empty strings, never shell syntax. `godot_args` are passed to Godot before the `--` separator; `project_args` are passed after the separator to the project/game. They may coexist on the same scenario and are reported separately in scenario command JSON, child run data, `meta.json`, `request.json`, runtime `result.json`, and bundle summaries. Invalid values fail before launch with `type: "scenario_validation_failed"` and an actionable payload.

`scenario run --json` separates process status, runtime result validation, log health, and final outcome under `data`:

- `data.godot_args` and `data.project_args`: effective scenario launch arguments as separate arrays. `data.project_args` reflects the values passed to Godot; persisted summary metadata redacts secret-looking project args as documented in the artifact format.
- `data.frame_pacing`: present for default fastest `scenario run` executions. It reports CLI-owned generated pacing metadata: `mode`, `fixed_fps`, `source`, `project_setting`, and `generated_godot_args`.
- `data.artifacts`: prepared and validated run artifact paths, including `run_root`, `request_path`, `result_path`, `logs_path`, `screenshot_paths`, and `diagnostic_paths` when available.
- `data.process`: CLI-owned Godot process summary with `command`, `command_line`, `exit_code`, and `status` (`passed` for exit `0`, otherwise `failed`). When the explicit wrapper-only policy applies, it also includes `warnings`.
- `data.runtime_result`: CLI validation summary for the runtime `result.json` payload with `path`, `status`, `ok`, and `failure_count`. `status` is `passed`, `failed`, `missing`, or `invalid`, so missing and malformed runtime payloads remain distinct from runtime assertion failures.
- `data.logs`: CLI-owned log health summary with `path`, `status`, `error_count`, and `warning_count`. Log errors and warnings remain distinct diagnostic states and fail strict scenario runs unless an existing documented log policy handles that line. The same object also reports `ignored_count` — how many `ERROR:`/`WARNING:` lines an ignore list silenced during that scan — and `ignore_patterns_applied`, the number of project `logs.ignore` patterns that were in force for the run, so a run that silenced engine noise never looks identical to one that had none. Both are `0` on the paths that report log health without scanning a log at all: a run timeout, or a missing or invalid runtime `result.json`. The project's `logs.ignore` list is applied on top of the built-in engine-noise ignores by cold runs, warm runs, and the warm session-noise scan alike, so cold and warm never disagree about what counts as noise. It is project-owned and it silences real errors: fix the game first and ignore only what the game cannot fix.
- `data.visual_risk`: present only when `--visual-risk` or `--visual-risk-strict` is requested. It contains `strict` plus `findings`, where each finding includes a stable `rule_id`, `severity`, measured facts under `actual`, and screenshot artifact paths under `artifacts` when available.
- `data.outcome`: final scenario decision. Strict passes use `{"status":"pass","reason":null}`. Failures use `{"status":"fail","reason":"<failure-type>"}`. The only warning pass status is `pass_with_warning`.

By default, `scenario run` and `scenario profile` use fastest frame pacing by launching Godot with generated `--fixed-fps <n>` and `--disable-vsync` flags. `determinism.fixed_fps` supplies `<n>` when present; otherwise the CLI derives it from positive `application/run/max_fps`, then `physics/common/physics_ticks_per_second`, then `60`. A scenario with `time_control.time_scale: 1.0` opts out for real-time soak profiling. Live sessions, recording, and non-manifest scene checks do not inherit this default.

If the scenario declares `time_control`, `scenario run --json` omits `data.frame_pacing` and instead includes `data.time_control` with Godot-owned `original`, `requested`, `effective`, `restored`, `restored_matches_original`, `restoration_status`, and `warnings` fields. The CLI validates `time_control.time_scale`, which the runtime applies as Godot `Engine.time_scale`, as a positive finite number `> 0.0` and `<= 64`, validates optional physics overrides as integers `>= 1`, and returns `type: "scenario_validation_failed"` before artifact creation or Godot launch when the schema is invalid. `time_control.time_scale: 1.0` is the persisted realtime opt-out. `time_control` and `determinism.fixed_fps` are mutually exclusive. Visual, screenshot-heavy, audio-sensitive, or physics-precision-sensitive accelerated flows without both paired physics settings report warning evidence such as `time_control_physics_pairing_missing` in `data.time_control_warnings` and, when the runtime applies settings, inside `data.time_control.warnings`.

Strict default semantics are unchanged: a scenario passes only when the Godot process exits `0`, runtime `result.json.ok` is `true`, runtime failures are empty, and relevant logs are clean. A clean runtime result plus a non-zero Godot process exit fails by default with `type: "godot_run_failed"` and `data.outcome.reason: "godot_run_failed"`. Missing `result.json` fails as `scenario_result_missing`; malformed or structurally invalid `result.json` fails as `scenario_result_invalid`; runtime assertion failures keep their runtime failure type; log errors remain `godot_log_error` or `godot_log_warning` failures.

`scenario run --visual-risk` enables optional screenshot artifact-backed risk analysis after the scenario has run and screenshot artifacts have been written. It inspects saved screenshot paths and visual assertion evidence for generic placeholder/blockout risk signals. Findings are warnings by default: the command can still exit `0`, top-level `ok` can remain `true`, and the warnings are added to `data.warnings` plus `data.visual_risk.findings`.

`scenario run --visual-risk-strict` enables the same analysis in strict mode. The JSON shape remains `data.visual_risk`, but findings are converted to error-severity failures and the command exits non-zero. Strict mode is the explicit opt-in required to make visual-risk findings fail automation.

Visual-risk findings are risk signals only. They do not judge art quality, approve or reject screenshot baselines, replace `screenshot_match`, replace screenshot-backed visual assertions, or change `screenshot_low_information`. Low-information screenshot failures remain stricter and keep `type: "screenshot_low_information"` outside the visual-risk namespace. Stable visual-risk rule IDs are:

- `visual-risk.low_visual_diversity`
- `visual-risk.huge_flat_shape`
- `visual-risk.debug_primary_colors`
- `visual-risk.missing_visible_target_pixels`

The only non-strict process outcome policy is the explicit wrapper-only opt-in declared in scenario metadata as `process_outcome_policy.known_wrapper_exit_codes`. It is considered only after the runtime result is present, valid, `ok: true`, runtime failures are empty, and logs are clean. A listed non-zero wrapper exit then keeps top-level `ok: true` and `failures: []`, records the process issue as a warning under `data.process.warnings`, returns `data.outcome.status: "pass_with_warning"`, and exits `0`. It must not mask crashes, script errors, runtime assertion failures, malformed or missing artifacts, log failures, absent policy, or unlisted exit codes.

Compact wrapper-warning example:

```json
{
  "ok": true,
  "command": "scenario run",
  "message": "scenario run passed with warning",
  "data": {
    "process": {
      "exit_code": 7,
      "status": "failed",
      "warnings": [
        {
          "type": "godot_run_failed",
          "severity": "warning",
          "expected": 0,
          "actual": 7
        }
      ]
    },
    "runtime_result": {
      "path": ".qa/artifacts/process_exit_warning_policy_demo/20260505T120000Z/result.json",
      "status": "passed",
      "ok": true,
      "failure_count": 0
    },
    "logs": {
      "path": ".qa/artifacts/process_exit_warning_policy_demo/20260505T120000Z/logs.txt",
      "status": "clean",
      "error_count": 0,
      "warning_count": 0
    },
    "outcome": {
      "status": "pass_with_warning",
      "reason": "godot_run_failed",
      "ok": true,
      "exit_code": 0
    }
  },
  "failures": []
}
```

`scenario lint` performs static scenario validation for manual-play integrity risk patterns and contract errors without launching Godot. By default, warning-only lint findings keep exit status `0`. `--strict` treats warning-level findings as failing for exit status without changing their `warning` severity. Contract errors (for example malformed files, malformed suppressions, duplicate scenario names, and statically provable missing scenario references) always exit non-zero in both modes.

`scenario lint` JSON payloads keep the standard envelope and expose stable finding lists under `data.findings` and `data.suppressed_findings`.

- `data.findings`: active lint findings after suppression, each with stable `rule_id`, `severity`, `scenario`, nullable `target` / `step_index` / `assertion_index` location fields, `reason`, and `suggested_fix`.
- `data.suppressed_findings`: warning findings hidden by valid suppressions; each entry keeps the same finding metadata plus suppression metadata so automation can report suppression use.

Example `scenario lint` output:

```json
{
  "ok": true,
  "command": "scenario lint",
  "message": "scenario lint passed",
  "data": {
    "summary": {
      "total": 1,
      "warnings": 1,
      "errors": 0,
      "strict": false,
      "suppressed": 1
    },
    "findings": [
      {
        "rule_id": "GQA-LINT-004",
        "severity": "warning",
        "scenario": "sample_main_menu_start",
        "scenario_path": "/path/to/project/qa/scenarios/sample_main_menu_start.yaml",
        "target": null,
        "step_index": null,
        "assertion_index": null,
        "reason": "interactive scenario has no negative check",
        "suggested_fix": "add at least one assertion that proves an unwanted state does not occur",
        "suppressible": true
      }
    ],
    "suppressed_findings": [
      {
        "rule_id": "GQA-LINT-002",
        "severity": "warning",
        "scenario": "lint_fixture_masking_demo",
        "scenario_path": "/path/to/project/qa/scenarios/lint_fixture_masking_demo.yaml",
        "target": null,
        "step_index": null,
        "assertion_index": null,
        "reason": "assertion checks debug text only",
        "suggested_fix": "pair debug text checks with player-visible assertions",
        "suppressible": true,
        "suppression": {
          "reason": "debug HUD is the public gameplay surface in this test fixture",
          "target": null
        }
      }
    ]
  },
  "failures": []
}
```

```json
{
  "ok": false,
  "command": "scenario run",
  "message": "scenario run failed",
  "data": {
    "scenario": "sample_settings_layout",
    "scene": "res://ui/settings_menu.tscn",
    "scenario_path": "/path/to/project/qa/scenarios/examples/sample_settings_layout.yaml",
    "duration_ms": 1450,
    "snapshots": ["before", "after_start"],
    "artifacts": {
      "run_root": "/path/to/project/.qa/artifacts/sample_settings_layout/20260421T120000Z",
      "request_path": "/path/to/project/.qa/artifacts/sample_settings_layout/20260421T120000Z/request.json",
      "result_path": "/path/to/project/.qa/artifacts/sample_settings_layout/20260421T120000Z/result.json",
      "logs_path": "/path/to/project/.qa/artifacts/sample_settings_layout/20260421T120000Z/logs.txt",
      "screenshot_paths": ["/path/to/project/.qa/artifacts/sample_settings_layout/20260421T120000Z/snapshots/before/screenshot.png"],
      "diagnostic_paths": ["/path/to/project/.qa/artifacts/sample_settings_layout/20260421T120000Z/snapshots"]
    },
    "snapshot_artifacts": {
      "before": {
        "root": "/path/to/project/.qa/artifacts/sample_settings_layout/20260421T120000Z/snapshots/before",
        "screenshot": "/path/to/project/.qa/artifacts/sample_settings_layout/20260421T120000Z/snapshots/before/screenshot.png",
        "tree": "/path/to/project/.qa/artifacts/sample_settings_layout/20260421T120000Z/snapshots/before/tree.json",
        "ui": "/path/to/project/.qa/artifacts/sample_settings_layout/20260421T120000Z/snapshots/before/ui.json",
        "focus": "/path/to/project/.qa/artifacts/sample_settings_layout/20260421T120000Z/snapshots/before/focus.json",
        "errors": "/path/to/project/.qa/artifacts/sample_settings_layout/20260421T120000Z/snapshots/before/errors.json"
      }
    },
    "screenshot_quality": {
      "before": {
        "path": "/path/to/project/.qa/artifacts/sample_settings_layout/20260421T120000Z/snapshots/before/screenshot.png",
        "width": 1920,
        "height": 1080,
        "pixel_count": 2073600,
        "dominant_colors": [{"rgba": [32, 32, 32, 255], "count": 120000, "ratio": 0.0578}],
        "non_background_pixel_count": 1953600,
        "non_background_ratio": 0.9421,
        "low_information": false,
        "low_information_reasons": []
      }
    },
    "launch_display": {
      "headless": false,
      "mode": "windowed",
      "platform": "linux",
      "display_driver": "x11",
      "wayland_available": false,
      "wayland_requested": false,
      "virtual_display": {
        "used": true,
        "wrapper": "xvfb-run",
        "command_prefix": ["xvfb-run", "-a"],
        "reason": "screenshot evidence requires rendered pixels and no host display was detected"
      }
    }
  },
  "failures": [
    {
      "type": "outside_viewport",
      "command": "scenario run",
      "target": "qa:apply_button",
      "severity": "error"
    }
  ]
}
```

If `result.json.artifacts` is present, it must be an object whose values are string paths. The CLI overlays those runtime-reported keys onto the prepared artifact map instead of replacing it, so `run_root`, `result_path`, `logs_path`, `screenshot_paths`, and `diagnostic_paths` remain available even when the runtime only reports a subset.
If `result.json.snapshot_artifacts` is present, it must be an object keyed by snapshot name, and each snapshot entry must also be an object of string paths. The CLI surfaces that validated nested map separately as `data.snapshot_artifacts` instead of folding it into the flat run-level `artifacts` object.
When scenario screenshots are saved, the CLI reads each screenshot artifact and adds `data.screenshot_quality.<snapshot>` with `width`, `height`, dominant RGBA color counts, `non_background_pixel_count`, `non_background_ratio`, and `low_information`. Low-information screenshots fail by default with `type: "screenshot_low_information"`. `snapshots.fail_on_low_information` remains accepted for compatibility, but transparent, blank, tiny, or visually flat screenshots are always failures.
When a screenshot-backed scenario cannot use an existing display and cannot safely select an available virtual display wrapper, `scenario run` fails before runtime execution with `type: "display_backend_unavailable"` (`display_backend_unavailable`). That failure is CLI-owned, distinct from `godot_launch_failed`, `scenario_result_missing`, `scenario_result_invalid`, runtime script errors, and assertion failures. If an artifact root has already been prepared, both `data.artifacts` and `failure.artifacts` include the run-level `run_root`, `result_path`, `logs_path`, `screenshot_paths`, and `diagnostic_paths`. The failure `expected` field describes a real display backend or detected virtual display wrapper, while `actual` records the host display environment and unavailable backend state.

Current runtime support is intentionally scoped to the implemented spec set:

- scenario files: JSON always; the built-in YAML subset works without extra dependencies, while broader YAML syntax still requires `PyYAML`
- steps: `wait_frames`, `snapshot`, `tap_action`, `hold_action`, `release_action`, `click`
- assertions: `exists`, `not_exists`, `visible`, `hidden`, `enabled`, `disabled`, `focused`, `text_equals`, `text_contains`, `inside_viewport`, `no_overlap`, `no_errors`, `visible_pixels`, `saved_snapshot_pixels`, `not_occluded`, `renders_above`, `renders_below`
- step-local `assert` blocks are supported and execute immediately at that point in `steps`; top-level `assert` blocks execute after all steps
- `qa:` target resolution uses the shared fast group lookup and fallback scan described in Target locators.
- Use `qa:` locators as durable anchors, not as per-object tags for every dynamic gameplay fact. Dynamic state should come from project-owned `get_*_qa_state()` or `get_qa_*_state()` methods for aggregate gameplay state, named `qa_*_probe()` methods for bounded probes, or screenshot-backed assertions for rendered perception; debug labels are diagnostics unless their visible text is the user-facing contract. Dynamic state should come from project-owned `get_qa_*_state()` methods, named `qa_*_probe()` methods, or screenshot-backed assertions when that older aggregate naming already fits the project.
- snapshot artifacts: each `snapshot` step writes `screenshot`, `tree`, `ui`, `focus`, and `errors` files and exposes them under `data.snapshot_artifacts.<name>`
- `visible` and `hidden` evaluate effective `CanvasItem` tree visibility, so a hidden parent makes its children count as hidden
- visual assertions such as `visible_pixels` are screenshot-backed checks for rendered 2D/UI evidence; they request screenshot artifacts, fail low-information screenshots as `screenshot_low_information`, and write diagnostics under `.qa/artifacts` without approving or updating `qa/baselines`
- `qa:`, `path:`, and `name:` locators must resolve uniquely; ambiguous matches fail with `type: "ambiguous_target"` and include the matching `path:/root/...` nodes in `actual`
- unsupported runtime step/assertion failures include the rejected operation name in `target`, the supported operation set in `expected`, and the rejected step/assertion object in `actual`
- malformed scenario top-level fields are rejected during CLI validation instead of being normalized away: `viewport`, `determinism`, and `snapshots` must be objects when present, and `assert` must be a list
- malformed payloads for supported operations are also rejected during CLI validation before artifact preparation or Godot launch:
  - `wait_frames` must be a non-negative integer
  - `snapshots.fail_on_low_information`, when present, must be a boolean
  - `tap_action`, `release_action`, `click`, and `exists|not_exists|visible|hidden|enabled|disabled|focused|inside_viewport` must be non-empty strings
  - `hold_action` must be an object with non-empty string `action` plus non-negative integer `frames`
  - `text_equals` and `text_contains` must be objects with non-empty string `target` plus string `expected`
  - `no_overlap` must be an object with non-empty string `a` plus non-empty string `b`
  - `no_errors` must be a boolean
- screenshot-producing commands require windowed Godot sessions; headless live sessions reject `snapshot`, `screenshot capture`, and `screenshot compare --live` as `headless_screenshot_unsupported`
- the intentionally bad layout demo should run separately from the happy path, for example `godot-qa layout audit --project examples/sample_project --scene res://ui/fixtures/bad_settings_menu.tscn --json || true`

`scenario record <name> --scene <res://...>` supports both manual and bounded non-interactive recorder flows:

- manual mode: launches a live session, captures `initial`, waits for `Ctrl+C`, captures `final`, exports the trace, writes a candidate scenario JSON file, and stops cleanly
- bounded mode: `--input-script <path> --duration-frames <n>` replays a checked-in input script for a fixed frame budget and exits on its own without waiting for a human signal

By default it writes:

- candidate scenario: `qa/scenarios/candidates/<name>.json`
- recorder artifacts: `.qa/artifacts/recordings/<name>/<run-id>/`

It rejects duplicate scenario names under `qa/scenarios` and rejects an existing output file instead of silently overwriting anything. In `--json` mode, malformed recorder input scripts return structured validation failures (non-zero) rather than partial artifacts. The generated result keeps `assert` empty and stores conservative review hints under `review.suggested_assertions`.

Recorder suggestions are review material only: use them to decide what to accept, reject, or edit manually. They complement `scenario lint` and do not replace lint warnings, do not auto-populate `assert`, and do not approve or write baselines.

The recorder also captures raw gamepad input so a human playtest becomes a replayable candidate. `InputEventJoypadButton` presses are reverse-mapped through the same named button table `input.joy_button` validates and exported with tap/hold pairing (a release within one frame becomes a one-frame `input_joy_button` tap; a longer press keeps `frames` equal to the observed span, which replays as press-hold-release). `InputEventJoypadMotion` values are quantized so analog noise does not produce thousands of steps: an `input_joy_motion` event is recorded only when the axis crosses a value bucket (`--joy-motion-step`, default `0.25`), returns to the deadzone (`--joy-motion-deadzone`, default `0.1`, recorded as `0.0`), or drifts to a new value after a minimum per-axis frame gap (`--joy-motion-frame-gap`, default `6` frames); the final value of every touched axis and any still-held button is always flushed at recording end. Frame gaps between recorded joypad events are exported as interleaved `wait_frames` steps so the candidate replays on the human's timeline. Unsupported button/axis indexes are reported once per index under `unconverted_events` instead of being guessed.

The shared bridge-compatible runtime command names for this spec are:

```text
tree.dump
ui.dump
focus.get
input.tap_action
input.hold_action
input.release_action
input.click_target
assert.exists
assert.not_exists
assert.visible
assert.hidden
assert.enabled
assert.disabled
assert.focused
assert.text_equals
assert.text_contains
assert.inside_viewport
assert.no_overlap
assert.no_errors
assert.visible_pixels
assert.not_occluded
assert.renders_above
assert.renders_below
```

The top-level live CLI commands (`godot-qa tree`, `godot-qa ui`, `godot-qa focus`, `godot-qa input ...`) now route through the S8 session transport and reuse the command names above instead of inventing a second runtime contract.

### S18 live inspection commands

```bash
godot-qa node inspect <target> --project examples/sample_project --json
godot-qa focus graph --project examples/sample_project --json
godot-qa errors --project examples/sample_project --json
```

- `godot-qa focus` still returns the current focus-owner payload backed by `focus.get`.
- `godot-qa focus graph` routes to `focus.graph` and returns the same bounded graph structure used by `focus.json`.
- `godot-qa node inspect <target>` routes to `node.inspect` and returns:
  - the requested `target`
  - the resolved stable `path:/root/...` locator
  - one bounded node payload with base identity fields, `locator`, `script_path`, one-hop `parent`, direct `children`, and a `control` block when the node is a `Control`
- `godot-qa errors` routes to `errors.read` and returns the structured QA runtime-error buffer already used by `errors.json` and `assert.no_errors`.

Example `node inspect` success payload:

```json
{
  "ok": true,
  "command": "node inspect",
  "message": "live command completed",
  "data": {
    "target": "qa:start_button",
    "resolved": "path:/root/MainMenu/Canvas/MenuButtons/StartButton",
    "node": {
      "locator": "qa:start_button",
      "path": "/root/MainMenu/Canvas/MenuButtons/StartButton",
      "name": "StartButton",
      "type": "Button",
      "qa_id": "start_button",
      "script_path": "",
      "parent": {
        "locator": "qa:main_menu_buttons",
        "path": "/root/MainMenu/Canvas/MenuButtons",
        "name": "MenuButtons",
        "type": "VBoxContainer",
        "qa_id": "main_menu_buttons"
      },
      "children": [],
      "control": {
        "text": "Start Session",
        "focus_mode": "all"
      }
    }
  },
  "failures": []
}
```

These surfaces are intentionally bounded. `node inspect` is not arbitrary property browsing, and `errors` does not scrape all Godot engine output.
`node inspect` target failures use the shared Target locators contract: stable `target_not_found` or `ambiguous_target` types, deduplicated `path:/root/...` ambiguity lists, and optional `candidate_status`/`rejected_candidates` diagnostics for rejected fast-path group candidates.

### Live physics inspection

```bash
godot-qa physics inspect <target> --project examples/sample_project --json
godot-qa physics inspect qa:physics_probe --project examples/sample_project --radius 300 --frames 2 --state-method get_qa_physics_state --snapshot collider-debug --json
```

`physics inspect` is a live-only diagnostic command backed by `physics.inspect`. It resolves `<target>` through the shared target resolver and returns bounded JSON evidence for invisible or mismatched 2D collision:

- target path/type, global position, rotation, velocity when available, visible rect when available, frame/time, and layer/mask when the target is a `CollisionObject2D`
- nearby `CollisionObject2D` shape-owner facts including shape type, disabled state, one-way state, layer/mask, approximate global AABB, owner path, transform, and distance from the target
- bounded `PhysicsDirectSpaceState2D.intersect_shape` radius and target-shape overlap query results
- diagnostic classifications such as `no_engine_target_shape`, `hidden_collision_object`, `large_shape_vs_visual`, `layer_mask_cannot_collide`, and `project_state_available`
- optional project-owned read-only QA state from safe zero-arg methods such as `get_qa_collision_footprint_state` and `get_qa_physics_state`

Options:

- `--radius <px>` defaults to `256` and must be `> 0` and `<= 4096`.
- `--frames <n>` defaults to `1` and must be between `1` and `240`.
- `--state-provider <locator|scene-root>` defaults to `scene-root`.
- `--state-method <name>` may be repeated and accepts only read-only `get_qa_*_state`, `get_*_qa_state`, or `qa_*diagnostic*` methods. Explicit state methods are preflighted before trace sampling; missing providers, missing methods, rejected methods, methods requiring arguments, runtime errors, and non-JSON-compatible values fail with structured errors and do not leave the live bridge busy.
- `--no-project-state` skips project QA state collection.
- `--snapshot <name>` captures a normal live snapshot next to the physics report.

Successful JSON includes `data.artifacts.physics`, written under `.qa/artifacts/live/<session-id>/physics/<capture-id>/physics.json`. When `--snapshot` is used, `data.artifacts.snapshot` contains the normal snapshot bundle paths. The command never adds visible debug UI or overlays.

### Physics collider overlays

```bash
godot-qa physics overlay qa:actor --project examples/sample_project --radius 400 --frames 2 --out .qa/artifacts/manual/physics_overlay_actor.png --json
godot-qa physics overlay --project examples/sample_project --viewport --out .qa/artifacts/manual/physics_overlay_viewport.png --json
godot-qa physics overlay qa:overlay_actor --project examples/sample_project --scenario physics_overlay_demo --snapshot aligned --out .qa/artifacts/manual/physics_overlay_from_snapshot.png --json
```

`physics overlay` renders artifact-only collider outlines over a PNG screenshot. Live mode captures a fresh base screenshot and collects collision metadata from the runtime bridge. Scenario review mode requires both `--scenario <name>` and `--snapshot <name>`; it reads `.qa/artifacts/<scenario>/latest/result.json` when available, otherwise the lexicographically newest run directory with `result.json`, and renders from `snapshots/<snapshot>/screenshot.png` plus matching `physics_overlays` metadata. Review mode fails with artifact errors when the screenshot or matching metadata is missing; it does not recapture live state.

Targeting and review options:

- A positional `<target>` such as `qa:actor` captures collision evidence near one resolved target.
- `--viewport` captures viewport-wide evidence and is mutually exclusive with a target.
- `--scenario <name> --snapshot <name>` switches to saved evidence review. Both flags are required together, and the command consumes the saved snapshot screenshot plus `result.json.physics_overlays` metadata from the selected scenario run.

Live collection options:

- `--radius <px>` defaults to `256`, must be `> 0` and `<= 4096`, and bounds nearby shape collection around a target.
- `--frames <n>` defaults to `1` and must be between `1` and `240`.
- `--include-aabb` asks the runtime/render path to include screen-space AABB boxes.
- `--include-contacts` asks for contact point/normal evidence when live physics contact data exists. Missing contact data is not guessed from sprites or scene files.
- `--state-provider <locator|scene-root>` defaults to `scene-root`.
- `--state-method <name>` may be repeated and accepts only read-only QA state/diagnostic providers such as `get_qa_collision_visual_state`, `get_qa_visual_footprint_state`, `get_*_qa_state`, or safe diagnostic `qa_*` methods.
- `--no-project-state` disables optional project visual-footprint/provider collection.
- `--max-shapes <n>` defaults to `512` and must be between `1` and `4096`; excess shapes are truncated deterministically and reported as a warning.
- `--strict` converts overlay warnings into a command failure.
- `--out <png>` is required. The path must end in `.png`.

Successful JSON uses command `physics.overlay` and reports the generated artifacts and warning summary:

```json
{
  "ok": true,
  "command": "physics.overlay",
  "data": {
    "target": "qa:actor",
    "artifacts": {
      "screenshot": "/abs/path/.qa/artifacts/manual/physics_overlay_actor.base.png",
      "overlay": "/abs/path/.qa/artifacts/manual/physics_overlay_actor.png",
      "metadata": "/abs/path/.qa/artifacts/manual/physics_overlay_actor.overlay.json"
    },
    "summary": {
      "shape_count": 3,
      "body_count": 2,
      "area_count": 1,
      "contact_count": 0,
      "visual_footprint_count": 1,
      "warnings": []
    }
  },
  "failures": []
}
```

The metadata JSON contains `legend`, rendered `shapes`, optional `contacts`, optional `project_state`, `runtime_metadata`, and the same warning objects. Legend entries name the rendered color/style for body collisions, area collisions, AABBs, contact points, contact normals, and project visual footprints.

Warning rule IDs are `physics_overlay.no_collision_shapes_found`, `physics_overlay.visual_footprint_without_collision`, `physics_overlay.collision_without_visual_footprint`, `physics_overlay.collider_extends_far_beyond_visual_footprint`, `physics_overlay.collider_center_offset_from_visual_footprint`, `physics_overlay.unbounded_shape_count_truncated`, and `physics_overlay.target_offscreen`. They are warning-level by default. With `--strict`, any warning returns a non-zero result with `kind: "physics_overlay_strict_warning"`.

Project visual-footprint providers are optional and must be read-only, zero-argument, JSON-compatible QA methods. Supported names follow the normal QA allowlist, for example `get_qa_visual_footprint_state()` or `get_qa_collision_visual_state()`, and may return `visual_footprints` entries with screen/world points, AABBs, shape facts, `solid_visual_body`, and `decorative_padding`. When no approved provider is available, visual-footprint comparison warnings are omitted rather than inferred from arbitrary sprites or textures.

Failure examples:

```json
{
  "ok": false,
  "command": "physics overlay",
  "message": "physics overlay requires --scenario and --snapshot together for review mode",
  "failures": [
    {
      "kind": "invalid_params",
      "command": "physics overlay",
      "expected": "both --scenario and --snapshot, or neither"
    }
  ]
}
```

```json
{
  "ok": false,
  "command": "physics.overlay",
  "message": "physics overlay strict validation failed",
  "failures": [
    {
      "kind": "physics_overlay_strict_warning",
      "command": "physics overlay",
      "expected": "no physics overlay warnings",
      "actual": ["physics_overlay.collider_extends_far_beyond_visual_footprint"]
    }
  ]
}
```

Overlay images are review artifacts only. They do not approve or update screenshot baselines, do not mutate collision shapes, and do not create a game-facing debug HUD.

### Live physics collision report

```bash
godot-qa physics collision-report qa:actor --project examples/sample_project --frames 600 --sample-every-frames 1 --physics --json
godot-qa physics collision-report --from-trace .qa/artifacts/live/<session-id>/traces/<trace-id>/trace.json --json
godot-qa physics collision-report qa:actor --project examples/sample_project --frames 600 --max-normal-retention 0.35 --max-velocity-reversals 2 --max-same-contact-frames 120 --json
```

`physics collision-report` summarizes collision-feel evidence for a bounded live target window or for an existing `trace record --physics` artifact. `--setup-script <path>` may be supplied before live capture, and `--summary-json` keeps stdout to summaries and artifact paths while still writing the full report and trace artifacts. The report is evidence for human playability review; it does not declare high bounce to be a game-specific failure by default because pinball-like behavior can be intentional. The command exits non-zero only for command/runtime errors or when callers opt into failing semantics with explicit thresholds or `--strict-warnings`.

Threshold flags:

- `--max-normal-retention <float>`
- `--max-velocity-reversals <int>`
- `--max-same-contact-frames <int>`
- `--max-angular-velocity-spike <float>`
- `--max-overlap-depth <float>`
- `--max-low-progress-contact-frames <int>`
- `--strict-warnings`

When `--strict-warnings` is supplied, any warning becomes a structured command failure. Without strict mode, threshold findings are reported under `data.summary.warnings` while the command can still return `ok: true`.

Output summary shape:

```json
{
  "ok": true,
  "command": "physics.collision_report",
  "data": {
    "target": "qa:actor",
    "frames_sampled": 600,
    "artifacts": {
      "report": "/abs/path/.qa/artifacts/live/<session-id>/collision-reports/<report-id>/collision_report.json",
      "trace": "/abs/path/.qa/artifacts/live/<session-id>/traces/<trace-id>/trace.json"
    },
    "summary": {
      "contact_frame_count": 123,
      "velocity_sign_reversal_count": 1,
      "same_contact_max_consecutive_frames": 18,
      "warnings": [
        {"rule_id": "collision_report.pinball_like_rebound", "severity": "warning"}
      ]
    },
    "metrics": {}
  }
}
```

Approved optional project provider names are `get_qa_collision_state()`, `get_collision_qa_state()`, `get_qa_physics_state()`, `get_physics_qa_state()`, and `qa_collision_probe()`. Providers must be zero-argument, read-only, JSON-compatible, and bounded. If no provider is available, the command still reports generic trace/physics metrics, emits `collision_report.provider_missing` when relevant, and marks provider-specific fields as `not_evaluated`; it must not guess soft, hard, or trigger semantics from node names.

Warning rule IDs include `collision_report.pinball_like_rebound`, `collision_report.repeated_bounce_loop`, `collision_report.same_contact_pin_risk`, `collision_report.angular_impulse_spike`, `collision_report.normal_velocity_retention_high`, `collision_report.velocity_reversal_high`, `collision_report.soft_contact_behaves_hard`, `collision_report.overlap_without_recovery`, `collision_report.provider_missing`, `collision_report.provider_invalid`, and `collision_report.insufficient_motion_data`. `collision_report.soft_contact_behaves_hard` is evaluated only when provider evidence explicitly classifies a contact as soft.

### Live runtime trace recording

```bash
godot-qa trace record --project examples/sample_project --target qa:trace_probe --frames 60 --sample-every-frames 1 --physics --json
```

`trace record` is a live-only timeline diagnostic backed by `trace.record`. It records observed runtime state, not replayable input and not video. `--setup-script <path>` may be supplied to run a deterministic live-session setup before recording. `--summary-json` omits bulky `samples` from stdout while preserving the full `trace.json` artifact path. It samples one or more live targets over a bounded frame window so agents can inspect how position, velocity, contacts, diagnostics, and project-owned QA state changed between frames.

Every `--setup-script` execution is bounded by a wall-clock deadline (default `120` seconds; all commands that accept `--setup-script` enforce this default) so a setup step that spawns a live session which never becomes reachable cannot hang the command indefinitely. On `trace record`, `--setup-timeout-seconds <n>` overrides the default. When the deadline is exceeded, any live session the setup script started is stopped and the command exits nonzero with `type: "setup_script_timeout"`, naming the script, the configured `setup_timeout_seconds`, and the elapsed time.

Options:

- `--target <locator>` is required and may be repeated.
- `--setup-timeout-seconds <n>` bounds `--setup-script` execution; defaults to `120` seconds.
- `--frames <n>` defaults to `180` and must be between `1` and `3600`.
- `--sample-every-frames <n>` defaults to `1` and must be between `1` and `240`.
- `--physics` includes compact `physics inspect`-style collision summaries for each target sample.
- `--state-provider <locator|scene-root>` defaults to `scene-root`.
- `--state-method <name>` may be repeated and accepts only read-only `get_qa_*_state`, `get_*_qa_state`, or `qa_*diagnostic*` methods.
- `--snapshot-start` and `--snapshot-end` optionally capture normal live snapshot bundles around the trace.
- `--stop-after-state '<json>'` stops recording early once a read-only `state_matches` predicate becomes true, using the same predicate shape as `audio capture --stop-after-state` (`target`, `source`, `method`, and either `matches` or `path`/`operator`/`expected`, with optional `sample_every_frames`). The predicate is polled once per captured sample. When it matches, the trace ends at that sample; when it never matches before `--frames` elapses, the command still writes the trace evidence and exits nonzero with `trace_record.stop_condition_timeout`. Invalid JSON, a missing `target`, or no `matches`/`path`+`operator` returns `invalid_params` before the session is contacted.

Successful JSON includes `data.artifacts.trace`, written under `.qa/artifacts/live/<session-id>/traces/<trace-id>/trace.json`. The trace contains session metadata, sample config, resolved targets, samples, derived events, and summary counts. Derived event types include `large_backward_displacement`, `velocity_sign_reversal`, `sudden_speed_spike`, `new_overlap`, `hidden_collider_nearby`, `layer_mask_mismatch`, and `project_state_changed`. When `--stop-after-state` is supplied, the payload also echoes `data.stop_after_state` (`requested`, `matched`, `sample_count`, `stopped_at_frame`, `last_error`), `data.stop_condition_matched`, and `data.stop_condition_timed_out`. The command never adds visible debug UI or overlays.

`trace record --events-provider` is not implemented in the current public command surface. Existing `trace record` behavior is unchanged when no event provider is supplied; use `event trace` as the supported generic event stream path for VFX/SFX/UI/collision correlation.

### Runtime event tracing

```bash
godot-qa event trace --project examples/sample_project --target qa:event_source --provider get_qa_event_stream_state --frames 120 --sample-every-frames 1 --json
godot-qa event summarize .qa/artifacts/live/<session-id>/events/<event-trace-id>/events.json --json
```

`event trace` records bounded, project-provided runtime events from an approved read-only provider. `--setup-script <path>` may be supplied before recording, and `--summary-json` omits bulky `events` from stdout while preserving `events.json` and `summary.json` artifact paths. As a compatibility alias, `--provider` may equal the `--target` locator; the CLI normalizes that form to `get_qa_event_stream_state`. It is the supported generic path for correlating gameplay, VFX, SFX, UI, collision, and custom QA events without visible debug labels or runtime overlays. The tool does not infer events from node names, logs, audio internals, VFX internals, hidden debug labels, or screenshots when no provider exists.

Options:

- `--project <project>` selects the Godot project.
- `--target <locator>` is required and identifies the provider target.
- `--provider <method>` is required and must be one of `get_qa_event_stream_state`, `get_event_stream_qa_state`, `get_qa_events`, or `qa_event_stream`.
- `--frames <n>` bounds the capture window.
- `--sample-every-frames <n>` controls sampling cadence.
- `--pairing-config <path>` optionally supplies project-agnostic `pairs` and `rate_limits` rules.
- `--strict-warnings` turns event trace warnings into command failures.
- `--json` emits the machine-readable result envelope.

Provider methods are read-only observation surfaces. They must return JSON-compatible event arrays or objects containing event arrays, remain bounded by the requested sample limits, and either return only new events since the last sample or include stable event IDs so the CLI can deduplicate. Event type strings are project-provided and are not interpreted or hardcoded by the tool; pairing and rate-limit rules compare only the strings supplied by the project or config.

Successful `event trace --json` includes:

```json
{
  "ok": true,
  "command": "event trace",
  "data": {
    "artifacts": {
      "events": ".qa/artifacts/live/<session-id>/events/<event-trace-id>/events.json",
      "summary": ".qa/artifacts/live/<session-id>/events/<event-trace-id>/summary.json"
    },
    "summary": {
      "total_event_count": 0,
      "event_count_by_category": {},
      "event_count_by_type": {},
      "event_rate_per_second_by_type": {},
      "max_events_in_one_frame": 0,
      "duplicate_event_count": 0,
      "missing_event_id_count": 0,
      "event_spam_warnings": [],
      "paired_event_warnings": [],
      "offscreen_event_warnings": [],
      "provider_error_count": 0
    },
    "warnings": []
  },
  "failures": []
}
```

`event summarize <events.json>` reads an existing event artifact and writes or reports the same summary shape without rerunning Godot. Warnings become failures only with `--strict-warnings`; warning IDs such as `event_trace.event_spam_risk`, `event_trace.paired_event_missing`, or `event_trace.no_events_recorded` remain advisory without strict mode.

Required event trace warning IDs are `event_trace.provider_missing`, `event_trace.provider_invalid`, `event_trace.event_missing_id`, `event_trace.event_not_json_compatible`, `event_trace.event_stream_truncated`, `event_trace.event_spam_risk`, `event_trace.paired_event_missing`, `event_trace.offscreen_event_risk`, `event_trace.provider_error`, and `event_trace.no_events_recorded`.

### Runtime telemetry recording

```bash
godot-qa telemetry record --project . --provider difficulty --frames 600 --every 10 --json
godot-qa telemetry record --project . --target qa:hud --method qa_pacing_metrics --frames 600 --every 10 --out .qa/artifacts/manual/pacing.jsonl --json
```

`telemetry record` samples ONE read-only `qa_*` state source every N frames during a live session and writes the rows as a JSONL artifact — pacing/difficulty evidence for consumer-game gates. It is live-only and backed by the `telemetry.record` bridge command. The source is either `--provider <id>` (a registered read-only state provider, sampled through the provider registry) or `--target <locator> --method <name>` (a zero-argument, read-only method on the resolved node: `qa_*` without the mutating `qa_prepare_/qa_apply_/qa_mutate_/qa_set_` prefixes, `get_qa_*_state`, or `get_*_qa_state`).

- `--frames` (default `600`, range `[1, 3600]`) bounds the sampling window; `--every` (default `1`, range `[1, 240]`) is the sample interval in frames.
- Collection is memory-bounded in the game process: at most `3600` rows are collected and the recording reports `truncated: true` when the cap stops collection early. Failed samples increment `provider_error_count`, keep a `{"error": ...}` row, and surface bounded `telemetry.sample_failed` warnings without aborting the recording.
- Artifacts land under the live session root at `telemetry/<telemetry-id>/telemetry.jsonl` (one `{"frame", "t_ms", "value"}` object per line) plus `manifest.json` (`rows`, `provider`/`target`+`method`, `frames`, `every`, `truncated`, `provider_error_count`), and `telemetry/latest` links the newest recording. `--out <path>` writes an additional copy of the JSONL.
- The command result reports `row_count` and artifact paths instead of echoing raw rows, so long recordings stay readable in `--json` mode.

Unlike `doctor`, an addon status of `installable` is not enough for `scenario run`: the selected target project must already contain `addons/godot_qa`, because Godot executes `res://addons/godot_qa/qa_runner.gd` from that project.

When `scenario run` has already prepared an artifact directory, CLI-owned failures such as launch errors, missing `result.json`, or invalid `result.json` also include `failure.artifacts` so the relevant paths stay attached to the failure object itself. If `result.json` is missing, `scenario_result_missing.actual` includes the missing result path, log path, last relevant Godot log lines, and any detected crash signal or script backtrace from `logs.txt`. Runtime failures that already carry snapshot-local artifact paths keep those snapshot keys, and the CLI merges in the prepared run-level paths (`run_root`, `result_path`, `logs_path`, `screenshot_paths`, and `diagnostic_paths`) so one failure object still points at both the specific snapshot evidence and the enclosing run context. `scenario_result_invalid` covers malformed JSON, structurally invalid payloads such as a non-object `result.json` root, non-boolean `ok`, non-array `snapshots`, non-string `snapshots` entries, non-integer `duration_ms`, non-object `artifacts`, `artifacts` values that are not strings, non-object `snapshot_artifacts`, `snapshot_artifacts` entries that are not objects of string paths, `failures` arrays that contain non-object entries, failure objects whose `type`/`message`/`path`/`target`/`severity` fields are not strings, failure `artifacts` payloads that are not objects of string paths, and failed runtime payloads that do not include actionable failure objects.

Process diagnostics additionally expose `data.process.termination` and
`data.process.timing`, including native Unix signals and monotonic observations
of process startup, matching-result publication, exit, and cleanup. The
termination scope is `direct_child`: a configured wrapper's status is not proof
of its Godot child's status. Existing exit fields and timeout behavior stay
compatible. Cold runs add `data.cold_timing` with `artifact_prepare_ms`,
`request_prepare_ms`, and, when completed, `cli_finalize_ms`. Imports also expose
`data.process`, including termination evidence on `godot_import_failed`.
See [process and timing artifacts](artifact-format.md) for field definitions and
the distinction between observed offsets, durations, and engine timing.
Process-layer failure payloads quote their Godot launch commands as needed, so `godot_launch_failed`, `godot_run_failed`, and `artifact_write_failed` remain actionable when binary or path arguments contain spaces.
The recorded `meta.json.command` and `request.json.run.command` fields include the effective CLI overrides used for that run, quoted as needed so the stored invocation remains reproducible when paths contain spaces.
Auto-generated run IDs are UTC timestamp-based; if the same scenario is prepared again within the same second, `scenario run` appends a numeric suffix to keep artifact roots distinct instead of reusing the earlier directory.
If the selected `--project` root does not exist, or if the target project is missing `addons/godot_qa`, `scenario run` fails before artifact preparation and does not create `.qa/artifacts` as a side effect under that target path.
If artifact preparation itself fails after those preflight checks, or if `logs.txt` cannot be written after Godot exits, `scenario run` returns `type: "artifact_write_failed"` with the failing artifact path and the underlying OS error instead of crashing out of the JSON contract.

If the selected scenario does not exist, the YAML file needs features outside the built-in subset, Godot cannot launch, or `result.json` is missing/invalid, `scenario run` exits `1` with a structured failure such as:

```json
{
  "ok": false,
  "command": "scenario run",
  "message": "scenario run failed",
  "data": {},
  "failures": [
    {
      "type": "scenario_not_found",
      "command": "scenario run",
      "path": "/path/to/project/qa/scenarios",
      "message": "scenario 'missing_scenario' was not found",
      "expected": "existing scenario name under qa/scenarios",
      "actual": "missing_scenario"
    }
  ]
}
```

If more than one scenario file resolves to the requested name, `scenario run` also exits `1` before artifact preparation or Godot launch with `type: "scenario_name_ambiguous"` and the matching file paths in `actual`.

The repository's example scenarios are not a complete sample Godot project yet. In practice, use `--project /path/to/godot-project` and point the scenario at scenes that exist in that project.

The richer scenario execution result below remains the target shape for later specs once layout audits, screenshot comparison, and broader assertions exist:

```json
{
  "ok": false,
  "command": "scenario run",
  "data": {
    "scenario": "settings_menu_layout",
    "artifacts": {
      "result_path": ".qa/artifacts/settings_menu_layout/latest/result.json"
    }
  },
  "failures": [
    {
      "type": "outside_viewport",
      "target": "qa:apply_button",
      "viewport": [1280, 720],
      "rect": [1090, 690, 240, 64]
    }
  ]
}
```

## Live session commands

```bash
godot-qa session start
godot-qa session start --scene res://ui/main_menu.tscn --viewport 1280x720
godot-qa session start --scene res://ui/main_menu.tscn --viewport 1280x720 --headless
godot-qa session start --scene res://ui/main_menu.tscn --headless --fastest --fixed-fps 60
godot-qa session start --scene res://ui/main_menu.tscn --project-arg --seed --project-arg 123456
godot-qa session list --json
godot-qa session status
godot-qa session wait --frames 10
godot-qa session time-scale 4 --physics-ticks-per-second 240 --max-physics-steps-per-frame 16 --json
godot-qa session time-scale 1 --json
godot-qa session reload
godot-qa session start --hot-reload --headless
godot-qa session reload --script res://ui/main_menu.gd
godot-qa session reload --resource res://ui/theme.tres
godot-qa session watch
godot-qa session watch --debounce-ms 300
godot-qa session pause
godot-qa session resume
godot-qa session step --frames 1
godot-qa session profile --frames 60 --json
godot-qa session profile --seconds 1.5 --max-frame-samples 120 --top 10 --json
godot-qa session stop
```

`session start` uses the project `run/main_scene` from `project.godot` when `--scene` is omitted. An explicit `--scene` overrides the project main scene. If neither is available, the command fails with `type: "missing_main_scene"`. Repeatable `--project-arg <value>` flags are passed after Godot's `--` separator to the project/game, matching scenario `project_args` launch semantics.

`session start --fastest` launches the live session with CLI-owned fastest frame pacing by adding generated `--fixed-fps <n>` and `--disable-vsync` Godot flags before `--path`. Realtime remains the default for interactive live sessions. `--fixed-fps <n>` is valid only with `--fastest`; when omitted, `<n>` is derived from positive `application/run/max_fps`, then `physics/common/physics_ticks_per_second`, then `60`. Invalid fixed FPS values fail before launch with `type: "invalid_params"`. Successful JSON includes `data.frame_pacing` when fastest mode is active.

`session start` writes a session file under `.qa/session.json` after the bridge becomes reachable, and writes the same recoverable metadata to `.qa/artifacts/live/<session-id>/session.json`:

```json
{
  "protocol_version": "0.1",
  "session_id": "20260421T120000Z",
  "pid": 12345,
  "host": "127.0.0.1",
  "port": 43123,
  "scene": "res://ui/main_menu.tscn",
  "viewport": [1280, 720],
  "artifact_dir": ".qa/artifacts/live/20260421T120000Z",
  "request_path": ".qa/artifacts/live/20260421T120000Z/request.json",
  "token_file": ".qa/session.token",
  "started_at": "2026-04-21T12:00:00Z",
  "headless": false,
  "project_args": ["--seed", "123456"],
  "frame_pacing": {
    "mode": "fastest",
    "fixed_fps": 60,
    "source": "session",
    "project_setting": null,
    "generated_godot_args": ["--fixed-fps", "60", "--disable-vsync"]
  }
}
```

On Windows, saved session metadata also includes optional `process_identity`, an opaque unsigned 64-bit process-creation timestamp (Windows FILETIME). The same field is stored for the sidecar in `.qa/debug_host.json`. Unix records omit it, and older records without it remain readable. Windows process checks compare the saved creation identity as well as the PID: a reused PID is stale and must never be terminated as the original session. Legacy records without identity cannot authorize fallback process termination; an authenticated bridge stop can still succeed, otherwise restart the session to establish current metadata.

The token value itself lives only in `.qa/session.token` and should not be printed in normal human output. `frame_pacing` is omitted for default realtime sessions. `session status` and `session list` include the persisted `frame_pacing` metadata when present. Live commands first use the primary `.qa/session.json` pointer. If that file is missing, the CLI can recover from live artifact metadata while `.qa/session.token` still exists. When `--artifact-dir` is provided, the CLI treats it as either a live session root containing `session.json`/legacy `request.json`, or as an artifact root containing `live/*/session.json`, and uses the newest valid live session metadata found there.
`session start` launches windowed by default so live screenshot commands capture real viewport pixels. `--headless` remains available for non-visual live inspection and input automation, but any command that writes screenshot evidence fails against a headless session with `type: "headless_screenshot_unsupported"`.

`session.status` now includes runtime `paused` state under `data.runtime.paused`. `session.pause` and `session.resume` return `{"paused": true|false}`. `session.step` is distinct from `session wait`: it requires a paused live session, advances exactly `--frames N`, and returns `{"paused": true, "frames": N}`.

`session time-scale <value>` changes Godot `Engine.time_scale` for the active live session through the authenticated bridge command `session.set_time_scale`. The value must be a positive finite number `> 0.0` and `<= 64`; `1.0` returns the live session to realtime scale. Optional `--physics-ticks-per-second <N>` and `--max-physics-steps-per-frame <N>` arguments set `Engine.physics_ticks_per_second` and `Engine.max_physics_steps_per_frame`; each must be an integer `>= 1`. Successful JSON includes Godot-owned `original`, `requested`, `effective`, `persistent: true`, and `warnings` fields. The setting persists in the running live Godot process until changed again, the session scene or project changes it, or the session exits; unlike one-shot scenario/check `time_control`, it is not automatically restored after the command returns. Accelerated live sessions without both paired physics settings report `time_control_physics_pairing_missing` warning evidence.

`session profile --soak <spec.json|yaml>` starts a background sampler in the active session. The foreground CLI polls short authenticated status calls once per second; `--detach` returns the run ID immediately. `session profile --soak-status <run-id>` reads progress and `--soak-stop <run-id>` stops a run, leaving `complete: false`. Exactly one soak may run per session. The live session must have no fastest/fixed-fps launch pacing and `Engine.time_scale` must equal 1. Sampling occurs during ordinary process frames, leaving the bridge available. Soak artifacts are under `.qa/artifacts/live/<session-id>/profiles/<run-id>/`. Session status reports runtime time scale, VSync mode, renderer and window size for provenance.

`session profile` samples an active live session for a short bounded window and writes `profile.json` under `.qa/artifacts/live/<session-id>/profiles/<profile-id>/`. By default it samples `60` frames; `--frames <n>` accepts `1..=1000`, and `--seconds <s>` may be used instead of `--frames`. `--setup-script <path>` may be supplied before profiling. When `--max-frame-samples` is omitted, the retained frame-sample limit follows the requested frame count for frame-based profiles, so `--frames 900` keeps up to 900 frame samples. `--summary-json` omits bulky sample arrays from stdout while preserving the full profile artifact. `--max-frame-samples` bounds retained frame and monitor history explicitly and remains capped at `1000`; `--top` bounds ranked sections. Invalid bounds fail before contacting Godot with `type: "invalid_params"`.

Successful JSON output uses `command: "session profile"` and includes session metadata, `data.sample_window`, `data.artifacts.profile`, and `data.profile` with frame-time percentiles, sampled Godot `Performance` monitor summaries, node process counts, resource activity, and `top_script_methods`. Method-level script samples are opt-in: project scripts can preload `res://addons/godot_qa/qa_profiler.gd` and wrap hot paths with `QaProfiler.begin("Label")` / `QaProfiler.end("Label")` or `QaProfiler.time("Label", callable)`. `session profile` resets those counters at the start of the sample window and reports ranked spans by total elapsed time; uninstrumented projects return `{"status":"available","instrumentation":"opt_in","methods":[]}`. Agents can read the same script-span surface from `data.profile.top_script_methods` for `session profile`, `scenario profile`, and `check --tests --profile`. Resource import/file-watch activity is `status: "unavailable"` unless live-session logs contain observable reload activity.

`session profile` may retry only before runtime sampling begins, and only for transient connection submission or bridge resource contention reported as `type: "session_bridge_busy"` or `session_connect_failed` with `Resource temporarily unavailable` / `os error 11`. The retry policy is bounded and visible in JSON through retry metadata such as `data.retry.attempts`, `data.retry.backoff_ms`, `data.retry.exhausted`, observed retryable failures, backoff milliseconds, `data.retry.final_status`, and sample window. Successful retry paths keep this metadata next to the normal `data.sample_window` and `data.artifacts.profile`; exhausted retry paths include the same metadata in the failure payload when available.

Profile failures use distinct kinds so agents can choose the right next action: `session_bridge_busy`, `session_unreachable`, `session_token_invalid`, `session_connect_timeout`, `profile_result_invalid`, and `artifact_write_failed`. Agents may retry `session_bridge_busy` failures because they indicate temporary live-bridge contention before profiling started. Agents should treat `session_unreachable`, `session_connect_timeout`, and `session_token_invalid` as configuration/session problems to resolve by checking `session list --json`, `session status --json`, the running Godot process, host/port, and token state instead of looping. Agents should not retry `profile_result_invalid` or `artifact_write_failed` as bridge-busy failures; malformed runtime profile payloads and artifact write failures need runtime or filesystem diagnosis and preserve their reported command, expected state, actual state, and artifact path fields.

`session reload` reloads the active live session scene inside the existing Godot process through the bridge command `session.reload_scene`. It uses the scene path recorded by `session start`, reloads the `PackedScene` from disk with deep cache replacement, swaps in the new scene root, clears held input/live-edit/runtime-error state, preserves paused state, and returns:

```json
{
  "scene": "res://ui/main_menu.tscn",
  "old_root": "/root/MainMenu",
  "new_root": "/root/MainMenu2",
  "paused": false
}
```

If reload loading or instantiation fails, the old scene remains running and the command returns `scene_load_failed` or `scene_instantiate_failed`.

### Engine-faithful hot reload (`--hot-reload`)

`session start --hot-reload` enables Godot-editor-fidelity hot reload of scripts and resources. It launches a persistent debug-host sidecar (an internal `godot-qa` subcommand) that listens on localhost, then starts Godot with `--remote-debug tcp://127.0.0.1:<port>` so the game connects to the sidecar over Godot's remote-debugger protocol. The sidecar holds that connection for the session lifetime, immediately disables error-break/breakpoint blocking so a script error can never freeze the game, and exposes a small control channel that the transient `session reload`/`session watch` processes use to request reloads. The sidecar is recorded in `.qa/debug_host.json` and reaped by `session stop`; `session status` reports a `debug_host` block and `hot_reload: true/false`. Because `--remote-debug` changes engine behavior, hot reload is opt-in.

With a `--hot-reload` session:

- `session reload --script <res://…>` (repeatable) performs an engine-faithful, state-preserving soft reload of the given scripts (`reload_scripts`): live instances keep their member state while their compiled code is swapped underneath them.
- `session reload --resource <res://…>` (repeatable) live-reloads the given resources (`scene:reload_cached_files`). This requires a debug/editor Godot binary (`DEBUG_ENABLED`); it is unavailable in `template_release` exports (silently ignored there), while script reload works in any build. Because the reload is fire-and-forget over the debugger protocol (no acknowledgement), a resource reload is reported as best-effort and unconfirmed: the result carries `status: "requested"` with the requested paths under `data.requested` (not `applied`) plus a `data.caveat` string, whereas a script reload reports `status: "applied"` with `data.applied`.
- `session reload --scene <res://…>` (repeatable) performs faithful `.tscn` **structural** live-editing: the debug-host sidecar diffs the scene against a baseline snapshot (captured at session start) and applies the delta — nodes added (`scene:live_create_node`/`live_instantiate_node`), removed (`live_remove_node`), reparented/renamed (`live_reparent_node`, state-preserving), and property values changed (`live_node_prop`/`live_node_prop_res`) — to every live instance without restarting. Like resource reload it needs a debug/editor build (`DEBUG_ENABLED`) and reports `status: "requested"` with a `data.caveat`, plus `data.ops_applied` (the op count) and `data.applied` (scenes applied live). Edits that cannot be expressed as live edits (an in-place node type change, a scene-root identity change, a `SubResource`-valued property, an unsupported value, or a scene with no baseline snapshot) are listed in `data.fell_back`; the caller full-scene-reloads those instead. Node renames/type-changes that can't be matched preserve no state (an inherent limit of file-diff live-edit vs. the editor's UndoRedo identity).
- `session reload` with no targets is unchanged (full-scene re-instantiation). `--script`, `--resource`, and `--scene` may be combined; they are applied in that order.

Granular reload without a `--hot-reload` session fails with `type: "hot_reload_not_enabled"`; if the sidecar has died it fails with `type: "hot_reload_unavailable"`.

`session watch` is a filesystem watch loop for live-session development. It uses native filesystem events when available and falls back to polling when the OS watcher cannot be registered. It requires an existing live session and reloads after relevant project files change. The default debounce is 300 ms and can be changed with `--debounce-ms <ms>`. It watches `.gd`, `.tscn`, `.scn`, `.tres`, `.res`, `.shader`, and `.gdshader` files while ignoring generated/noisy paths such as `.git/`, `.godot/`, `.qa/`, legacy `qa/artifacts/` and `qa/tmp/`, `target/`, `build/`, and `dist/`. When the session was started with `--hot-reload`, it classifies each change: `.gd` -> script soft reload, resource types (`.tres`, `.res`, `.gdshader`, `.shader`, `.scn`) -> live resource reload, and `.tscn` -> faithful structural live-edit (`reload_structure`). A `.tscn` change whose delta cannot be applied live emits a `reload_notice` with the `fell_back` reasons and then a full-scene reload (which resets state) for that scene. Without a debug host it falls back to full-scene reload for every change (the pre-hot-reload behavior) and reports `hot_reload: false` in `watch_started`. On the first resource reload of a watch session it emits a one-time `resource_reload_caveat` event (the resource reload is best-effort and needs a debug/editor build). In `--json` mode it emits newline-delimited JSON events: `watch_started`, `change_detected`, `reload_succeeded`, `reload_failed`, `reload_notice`, `resource_reload_caveat`, and `watch_stopped`; reload events carry a `reload_kind` (`reload_scripts`, `reload_resources`, `reload_structure`, or `full_scene`) and the affected `paths`.

Invalid or unsupported session-control transitions stay machine-readable:

- `session step --frames 0` or a negative value returns `type: "invalid_params"`
- `session time-scale 0`, a negative value, NaN, infinity, or values above `64` return `type: "invalid_params"`
- `session time-scale <value> --physics-ticks-per-second 0` or `--max-physics-steps-per-frame 0` returns `type: "invalid_params"`
- pausing an already paused session returns `type: "session_already_paused"`
- resuming or stepping while not paused returns `type: "session_not_paused"`

The live artifact root for that session is:

```text
.qa/artifacts/live/<session-id>/
  meta.json
  session.json
  request.json
  logs.txt
  profiles/
    <profile-id>/
      profile.json
  physics/
    <capture-id>/
      physics.json
  traces/
    <trace-id>/
      trace.json
  snapshots/
    <snapshot-name>/
      screenshot.png
      tree.json
      ui.json
      focus.json
      errors.json
```

`session status` combines the saved session metadata with the runtime `session.status` payload. `session list` is a compatibility alias with the same metadata and live status lookup, but its JSON envelope reports `command: "session list"`. If `.qa/session.json` still points at a dead PID and the bridge is unreachable, `session status`, `session list`, and other live commands return `type: "stale_session"` instead of a generic connect error so automation can distinguish stale metadata from a live bridge outage. `session stop` waits for the recorded PID to exit before deleting `.qa/session.json` and `.qa/session.token`; if the bridge acknowledges the stop but the process stays alive, the command fails with `type: "godot_stop_failed"` and leaves temp files in place for follow-up cleanup. It still cleans stale metadata if the process is already gone, and falls back to terminating or killing the saved PID when the bridge is unreachable or reports live-command contention while the process is still alive.

## Snapshot commands

```bash
godot-qa visual preflight --project examples/sample_project --json
godot-qa assert visible-pixels qa:important_actor --snapshot occluded --min-ratio 0.25 --json
godot-qa session start --scene res://ui/main_menu.tscn --viewport 1280x720 --json
godot-qa snapshot --name before
godot-qa screenshot capture --out .qa/artifacts/manual/main_menu_720p_candidate.png --json
godot-qa screenshot capture --setup-script qa/review-scripts/prepare_windowed.yaml --out .qa/artifacts/manual/prepared.png --json
godot-qa screenshot compare --actual .qa/artifacts/current.png --baseline qa/baselines/main_menu.png --json
godot-qa screenshot compare --live --baseline .qa/artifacts/manual/main_menu_720p_candidate.png --json
godot-qa session stop --json
godot-qa tree
godot-qa tree --target qa:start_button --max-depth 1 --metadata-key qa_id --json
godot-qa ui
godot-qa focus
godot-qa logs --since 3
```

A live snapshot creates:

```text
screenshot.png
tree.json
ui.json
focus.json
errors.json
```

`snapshot` accepts the same `--target`, `--max-depth`, `--qa-only`, `--include-metadata`, `--metadata-key`, and `--metadata-keys` flags as `tree`. The snapshot directory and filenames remain stable (`screenshot.png`, `tree.json`, `ui.json`, `focus.json`, `errors.json`) whether the JSON evidence is full or filtered. Full unfiltered snapshots remain the compatibility and debugging path when no filter flags are supplied.

`logs` reads structured bridge/runtime entries from the live session and returns `entries` plus `next_cursor`. Raw Godot stdout/stderr still lives in the session-level `logs.txt` artifact.

`session run-script <path>` executes a declarative live-session script sequentially. The script is JSON/YAML data with a `steps` array; each step is a single operation such as `session_start`, `session_pause`, `session_resume`, `tree`, `ui`, `node_inspect`, `input_key`, `input_text`, `input_joy_button`, `input_joy_motion`, `call_method`, `fixture_call`, `screenshot`, `logs`, `errors`, `assert`, or `session_stop`. `input_text` accepts either a text string or `{text: string}`; `input_joy_button` accepts either a button string or `{button, optional device, optional frames}`; `input_joy_motion` accepts `{axis, value, optional device, optional frames}`. `pause_session` and `resume_session` are accepted aliases for `session_pause` and `session_resume`. `session_start` may include `project_args: [...]`, passed after Godot's `--` separator. It may also request fastest live pacing with either `frame_pacing: {mode: fastest, fixed_fps: 60}` or shorthand `fastest: true` plus optional `fixed_fps: 60`; omitted `fixed_fps` uses the same project-setting derivation as `session start --fastest`. If a script reuses an already-running session, the existing session's pacing is reported; a requested fastest pacing mismatch fails with `type: "session_frame_pacing_mismatch"` instead of silently reusing the session. When `session_start` requests a windowed session but the existing session is headless, the runner stops the incompatible session and starts a windowed one instead of reusing it for screenshot-producing steps. `call_method` dispatches the same live `setup.call_method` fixture operation as saved scenarios and only allows project-owned methods beginning with `qa_`; `fixture_call` dispatches the same setup-only `fixture.call` registered driver operation as saved scenarios and uses `driver`, `action`, and optional object `params`. The runner stops at the first failed step and returns per-step JSON evidence, which avoids overlapping live-bridge requests and the resulting `session_bridge_busy` failures. `session.wait_frames` requests use a frame-derived timeout of `max(5s, ceil(frames / 30fps) + 5s)`, capped at 120 seconds, so long waits can complete without splitting scripts into many short waits. Transient live-bridge transport failures such as `session_bridge_busy` and `session_connect_failed` with `Resource temporarily unavailable (os error 11)` are retried before the step fails; assertion, target, timeout, and gameplay failures are not retried. When bridge-busy diagnostics identify an in-progress `session.wait_frames`, retry metadata includes `wait_aware: true`, `backoff_milliseconds`, and the in-progress wait diagnostics. Use `assert: {state_matches: {...}}` for read-only project QA state such as `get_qa_lobby_state()` instead of `edit-live call`.
Live scripts also accept `joy_connect` and `joy_disconnect` with the saved-scenario `{devices: [...]}` shape (optional `name` on connect), `session_change_scene` with a scene path or `{scene: path}`, and `session_reload_scene`. This permits a single bounded script to check virtual-pad cleanup across a scene replacement and session stop; see `examples/sample_project/qa/live_scripts/joy_lifecycle_cleanup_smoke.yaml`.
Direct live commands such as `snapshot`, `tree`, `ui`, `node inspect`, `physics inspect`, `surface audit`, `trace record`, `event trace`, `audio capture`, `session profile`, `errors`, `logs`, and live `assert` commands also perform a bounded retry when the bridge reports transient command contention: `session_bridge_busy`, `resource_unavailable`, `resource_busy`, `bridge_busy`, or `session_connect_failed` containing `Resource temporarily unavailable` / `os error 11`. Bridge-busy diagnostics that include an in-progress command window are used to choose a less aggressive retry delay. JSON output includes `data.retry` with attempts, backoff, observed retryable failures, final status, and exhaustion state when a retry was attempted. Assertion failures, target-resolution failures, malformed runtime payloads, invalid tokens, stale sessions, and other non-transient failures are not retried.
`screenshot capture` requires an active windowed live session. `--setup-script <path>` may be supplied to run a `session run-script` fixture/setup script before capture starts; setup scripts may include `session_start`, `call_method`, or registered-driver `fixture_call` steps. `--require-visible <locator>`, `--require-hidden <locator>`, and `--require-state '<json>'` are optional state guards evaluated immediately before capture; failed guards prevent capture and return structured diagnostics with current session status, visible anchor context, and the mismatched state assertion. Godot stages one PNG under the current session artifact root, the CLI quality-checks it, and the CLI copies it to the requested `--out` path only when the staged image is not low-information. Successful JSON includes `data.setup_script` when setup was requested and records passed state guards when supplied.

`screenshot assert-startup-presented` checks the pixels on a fresh, untouched window in an isolated compositor:

```bash
godot-qa screenshot assert-startup-presented --project <project> --scene res://ui/fixtures/presented_startup_demo.tscn --viewport 1280x720 --display-backend wayland --json
godot-qa screenshot assert-startup-presented --project <project> --scene res://ui/fixtures/presented_startup_demo.tscn --viewport 1280x720 --display-backend x11 --baseline qa/baselines/presented.png --threshold 0.01 --channel-tolerance 1 --json
```

Linux headless Gamescope and `gamescopectl` are required. The command owns a new Gamescope process and exactly one Godot live session, waits two idle seconds after session startup, saves `presented.png` before requesting any draw-forcing viewport screenshot or input, then saves `viewport.png` through a strict no-fallback bridge capture. Without `--baseline`, the presented image is compared with the later viewport image. With `--baseline`, it is compared with that existing PNG; this command never approves or updates baselines. `--channel-tolerance` is opt-in and defaults to exact per-channel comparison (`0`); a value of `1` treats a pixel as equal only when every RGBA channel differs by at most one 8-bit level. The 1% differing-pixel threshold is unchanged. Metrics report `raw_different_pixels` and `raw_different_ratio`, `tolerated_different_pixels` and `tolerated_different_ratio`, `channel_tolerance`, and the observed `max_channel_delta`; `different_pixels` and `different_ratio` are the count and ratio used for the threshold. The diff PNG marks raw pixel differences even when tolerance permits a pass, and the original PNGs remain byte-exact. This option applies only to this presented-startup assertion; ordinary `screenshot compare` stays exact. `--out <new-directory>` is optional; by default artifacts go to a unique `.qa/artifacts/presented-startup/<run-id>/` directory. `--display-backend` accepts `wayland` (default) or `x11`. The compositor output, viewport, comparison metrics, raw diff, capture provenance, wrapper log, and both original images are retained. JSON records the actual Godot display backend, session ID/PID, compositor identity, dimensions, monotonic capture order, draw-forced status, and SHA-256 hashes. Mismatched dimensions fail without scaling. A configured Godot launch prefix or display-driver override fails `presented_capture_unsupported`, since it could redirect the child outside the owned display; an already-running project session is never stopped by this check. The external Gamescope exit alone is not a pass verdict: the command checks the run-matched helper result and clean Godot teardown, then writes the parent’s finalized `result.json` for evidence export. A compositor capture is never replaced with viewport pixels.
`screenshot compare` still performs CLI-side image comparison. With `--live`, the CLI first requires a windowed live session, captures and quality-checks a staged PNG, and then reuses the same baseline, threshold, metrics, and diff logic as file-backed compare. Headless live sessions fail before capture.
`screenshot compare --live` requires `--baseline` and must not be combined with `--actual`.
`screenshot compare` and `screenshot compare --live` depend on image-comparison support compiled into the shipped CLI, while `screenshot capture` does not add extra install steps.

`screenshot sequence` captures bounded live-session screenshot series for review:

```bash
godot-qa screenshot sequence --project <project> --frames 300 --every 15 --out .qa/artifacts/manual/race_sequence --json
godot-qa screenshot sequence --project <project> --seconds 20 --fps 2 --out .qa/artifacts/manual/review_sequence --json
godot-qa screenshot sequence --project <project> --scenario screenshot_sequence_demo --frames 120 --every 20 --out .qa/artifacts/manual/scenario_sequence --json
godot-qa screenshot sequence --project <project> --setup-script qa/review-scripts/prepare_race.yaml --require-visible qa:race --require-hidden qa:lobby --require-state '{"source":"provider","provider":"countdown","matches":[{"path":"current_surface","operator":"equals","expected":"race"}]}' --frames 120 --every 20 --out .qa/artifacts/manual/race_sequence --json
```

The timing forms are mutually exclusive: use `--frames <count> --every <frames>` for fixed engine-frame windows, or `--seconds <seconds> --fps <rate>` for seconds/FPS capture. `--setup-script <path>` may be supplied to prepare a windowed live session before captures begin. `--require-visible`, `--require-hidden`, and `--require-state` use the same pre-capture guard semantics as `screenshot capture`, run before each frame, and are recorded in the sequence manifest so artifacts can prove their runtime state. `--scenario <name>` resolves a scenario document by declared name, reports `missing_scenario` or `scenario_name_ambiguous` before capture when needed, and records `scenario`, `scenario_path`, `scenario_scene`, and `scenario_steps_replayed: false` in the manifest. It does not fork or replay `scenario run` semantics, setup steps, assertions, or snapshot behavior.

`--require-state` and `--stop-after-state` are distinct: `--require-state` is a post-hoc pre-capture guard that must already hold for every frame (a mismatch fails the sequence), while `--stop-after-state '<json>'` stops the sequence early once a read-only `state_matches` predicate becomes true. `--stop-after-state` uses the same predicate shape as `audio capture --stop-after-state` and is polled once per captured frame. When it matches, capture ends at that frame (the manifest simply contains fewer frames); when it never matches before the capture plan completes, the command exits nonzero with `screenshot_sequence.stop_condition_timeout`. Invalid JSON, a missing `target`, or no `matches`/`path`+`operator` returns `invalid_params` before the session is contacted.

Sequence JSON reports `frame_count_requested`, `capture_every_frames`, `captured_count`, `low_information_frame_count`, `failed_capture_count`, `visual_risk_warning_count`, `first_failed_frame`, and artifact paths for `sequence_dir`, `frames_dir`, and `manifest`. When `--stop-after-state` is supplied it also echoes `data.stop_after_state` (`requested`, `matched`, `sample_count`, `stopped_at`, `last_error`), `data.stop_condition_matched`, and `data.stop_condition_timed_out`. Each frame reuses screenshot low-information checks; low-information frames fail unless `--allow-low-information` is supplied. `--visual-risk` adds per-frame warning evidence under `quality.visual_risk_warnings`; `--visual-risk-strict` converts those warnings into failures. These are risk signals only and do not approve visual quality or baselines.

`screenshot contact-sheet` builds a review grid from a sequence directory:

```bash
godot-qa screenshot contact-sheet .qa/artifacts/manual/race_sequence --columns 5 --thumb-width 320 --out .qa/artifacts/manual/race_contact_sheet.png --json
```

The contact sheet uses manifest order, preserves aspect ratio, places frame/time labels in artifact margins rather than altering screenshots, writes a companion JSON describing source image paths and grid geometry, and fails actionably for missing, unreadable, or unsupported image inputs. Baseline approval remains manual.

`visual audit` produces screenshot-backed readability and contrast evidence for explicit targets. It is a measurable review aid, not an aesthetic pass/fail oracle, and it never auto-approves baselines or replaces `screenshot compare`, visual-risk analysis, or visible-pixel assertions. Findings are warnings by default; `--strict-warnings` converts visual audit warnings into command failures.

```bash
godot-qa visual audit --project <project> --live --target qa:player --target qa:progress_ui --json
godot-qa visual audit --screenshot .qa/artifacts/manual/current.png --targets .qa/artifacts/manual/current.targets.json --json
godot-qa visual audit --sequence .qa/artifacts/manual/race_sequence/manifest.json --target qa:player --json
godot-qa visual audit --scenario visual_audit_demo --project examples/sample_project --run-id latest --target qa:readable_target --json
godot-qa visual audit --project <project> --live --target qa:player --annotate-out .qa/artifacts/manual/visual_audit.png --json
```

Exactly one source mode is required: `--live`, `--screenshot <png>`, `--sequence <manifest.json>`, or `--scenario <name-or-path>`. Live mode captures or reuses the active session screenshot and resolves supplied `--target <locator>` entries through the Godot runtime target evidence. Saved screenshot mode requires `--targets <json>` with explicit viewport-space bounds and must not infer object bounds from arbitrary sprite colors. Sequence mode reads a screenshot sequence manifest and summarizes matching targets across frames. Scenario artifact mode reads saved snapshot evidence from a prior scenario run, with `--run-id <id>` defaulting to the latest suitable run when supported.

Target bounds come from visible-pixel/screenshot target extraction when present, live QA target rect evidence, optional project-owned visual state methods (`get_qa_visual_state`, `get_visual_qa_state`, `get_qa_readability_state`), or explicit `--targets` JSON. Missing bounds produce `visual_audit.target_bounds_unavailable`; a missing-bounds case does not trigger fake target inference.

Default thresholds are conservative review thresholds: `min_pixel_area: 256`, `min_width: 16`, `min_height: 16`, `min_contrast_ratio: 3.0`, `min_color_distance: 24.0`, and `min_edge_contrast: 0.08`. Callers may override the public thresholds with `--min-pixel-area <value>`, `--min-width <px>`, `--min-height <px>`, `--min-contrast-ratio <float>`, and `--min-color-distance <float>`. Edge contrast currently uses the documented default threshold and is reported as `edge_contrast_estimate`.

`--json` reports `data.visual_audit.screenshot_path`, `width`, `height`, `targets`, `findings`, `thresholds`, `strict`, and `artifacts`. Per-target metrics include `target_id`, `screenshot_path`, `inside_viewport`, `pixel_area`, `bounding_box_width`, `bounding_box_height`, optional `visible_pixel_count`, `estimated_local_background_color`, `estimated_target_color_or_median`, `relative_luminance_target`, `relative_luminance_background`, `contrast_ratio_estimate`, `local_color_distance`, `edge_contrast_estimate`, `small_target_warning`, `low_contrast_warning`, `offscreen_warning`, optional `occlusion_or_missing_pixels_warning`, `target_rect`, and `clamped_rect`. Sequence audits also report `frames_checked`, `frames_with_target_missing`, `minimum_pixel_area_across_frames`, `minimum_contrast_across_frames`, and `worst_frame_path_by_rule`.

Required warning IDs are `visual_audit.target_missing`, `visual_audit.target_offscreen`, `visual_audit.target_too_small`, `visual_audit.low_luminance_contrast`, `visual_audit.low_color_distance`, `visual_audit.low_edge_contrast`, `visual_audit.target_bounds_unavailable`, `visual_audit.sequence_target_missing_frames`, and `visual_audit.annotated_artifact_failed`. Optional `--annotate-out <path.png>` writes an annotated PNG artifact with target boxes and warning marks outside or over the artifact image; it does not draw into the running game.

`visual isolate` is a screenshot-diff workflow for project-owned visual layer providers. The project supplies a registered state provider that enumerates named render contributors and a registered fixture driver/action that can set `normal`, `only <layer>`, and `without <layer>` modes without adding visible debug UI. The CLI captures a baseline, per-layer isolated screenshots, per-layer disabled screenshots, and diff images from the same prepared live state, then writes a contact sheet and manifest with changed-pixel bounds plus straight-edge/rectangular-region warnings.

Once fixture mode changes begin, the command always attempts to restore `normal`
mode, including after a failed fixture call, screenshot capture, or diff. The
original failure remains first. A failed restoration adds
`visual_isolation_cleanup_failed`, with the expected normal mode and the cleanup
command's failure evidence; successful captures with failed restoration also fail.
Unchanged images report zero changed pixels, zero changed-pixel ratio, and null
bounds. Captures are
sequential, so projects should prepare a stable visual state when animation would
otherwise contaminate their differences.

```bash
godot-qa visual isolate --project <project> --setup-script qa/review-scripts/prepare_race.yaml --provider race_visual_layers --driver race_review --action set_visual_layer_mode --out .qa/artifacts/manual/race_visual_isolation --json
godot-qa visual isolate --project <project> --provider race_visual_layers --layer water_surface --layer bank_shallow_gradient --out .qa/artifacts/manual/water_layers --contact-sheet-out .qa/artifacts/manual/water_layers.png --json
```

If `--layer` is omitted, the layer list comes from provider state at `layers[*].id` or object keys under `layers`. The provider state is included in JSON evidence. Diff warnings are review diagnostics only; they identify suspicious layer ownership and do not approve or reject visual design by themselves.

`visual preflight --project <project> --json` is the CLI-owned launch preflight for screenshot-backed visual assertions. It does not run a scenario or resolve a target; it reports whether the selected launch mode can provide real viewport pixels and whether runtime-dependent visual capabilities can be attempted. The JSON payload includes `data.project_root`, `data.headless`, `data.command_line`, `data.launch_display`, and `data.visual_support`. `visual_support` contains `real_viewport_pixels`, `target_hidden_capture`, `mask_generation`, and `saved_snapshot_object_pixels`; each capability reports `available`, `reason`, `expected`, `actual`, and `remediation`. Common reasons include `display_backend_available`, `virtual_display_available`, `forced_headless`, and `display_backend_unavailable`. Target-specific geometry and hook support is still validated by the scenario or live assertion that resolves the locator.

`assert visible-pixels`, `assert not-occluded`, `assert renders-above`, and `assert renders-below` use the same live screenshot evidence surface. They are appropriate for deterministic 2D/UI targets where the runtime can report a target rect or projected rect. Unsupported or ambiguous visual evidence fails with documented limitation payloads instead of approving a baseline.

`visible-pixels` and `not-occluded` can measure targets that expose an explicit viewport-space rect through `qa_visual_rect()` or `qa_visual_rect` metadata. Accepted rect shapes are `{x, y, width, height}`, `[x, y, width, height]`, or `Rect2`. Non-`CanvasItem` targets must also expose `qa_set_visual_visible(visible)` for target-hidden pixel-delta evidence. `Line2D` targets are measured from viewport bounds of the rendered stroke, not only the node origin.

Saved scenarios may use `saved_snapshot_pixels` when a prior saved snapshot already contains a real screenshot plus target `visual_evidence` in viewport coordinates. The direct CLI equivalent is `assert visible-pixels --scenario <scenario> --snapshot <snapshot> ...`, which loads the named saved snapshot instead of recapturing the whole scenario. Use it for object-pixel evidence in an existing run artifact; use live `assert visible-pixels` or scenario `visible_pixels` when the assertion must capture current runtime state.

Generated visual assertions should target player-visible, deterministic rendered evidence and include an explicit snapshot plus a threshold. Use `assert visible-pixels` for minimum rendered coverage, `assert not-occluded` for target-hidden delta evidence and occlusion diagnostics, and render-order assertions when relative draw order is the contract. For dynamic UI, debug overlays, or custom drawing where automatic geometry is insufficient, add project-owned QA hooks such as `qa_visual_rect()` and `qa_set_visual_visible(visible)`. If the desired contract is non-visual state, use a QA-prefixed state method or probe assertion instead of forcing a pixel assertion through debug labels.

Scenario and live-session JSON include `launch_display` metadata when Godot is launched by the CLI. The object records whether the run was headless, the selected launch `mode`, the host platform, detected `DISPLAY`/Wayland availability, whether the project requested Wayland, the explicit `--display-driver` value requested by `godot-qa` when one was applied, `launch_prefix_used`, `windowed_launch_prefix_used`, `effective_command_prefix`, and any selected `virtual_display` wrapper with its command prefix and selection reason. Runtime `display_server` remains the Godot-reported display backend.
Visual assertion failure payloads include the assertion `command`, `target`, `snapshot`, measured pixel evidence (`visible_pixel_count` and/or `visible_pixel_ratio`), configured threshold (`min_count` and/or `min_ratio`), and runtime geometry evidence (`target_rect` or `projected_rect`); `not-occluded` failures include `occluder` when known and `renders-above`/`renders-below` failures include `other`. Diagnostics are written as candidate artifacts under `.qa/artifacts` and never auto-approve or update `qa/baselines`.
Headless live sessions reject screenshot-producing visual assertions with `type: "headless_screenshot_unsupported"` before any pixel evidence is evaluated. Forced headless runs must not pass by using diagnostic fallback images; visual assertions require real rendered pixels from a display backend or supported virtual display.

Example CLI forms:

```bash
godot-qa visual preflight --project examples/sample_project --json
godot-qa assert visible-pixels qa:important_actor --project examples/sample_project --snapshot occluded --min-ratio 0.25 --json
godot-qa assert visible-pixels qa:saved_pixels_visible_target --project examples/sample_project --scenario saved_snapshot_pixels_demo --snapshot saved-pixels --min-ratio 0.45 --json
godot-qa assert not-occluded qa:important_actor --project examples/sample_project --snapshot occluded --occluder qa:debug_overlay --json
godot-qa assert renders-above qa:hud_label --other qa:hud_backplate --project examples/sample_project --snapshot hud --json
godot-qa assert renders-below qa:hud_backplate --other qa:hud_label --project examples/sample_project --snapshot hud --json
```

## Input commands

```bash
godot-qa input tap ui_accept
godot-qa input hold ui_down --frames 4
godot-qa input release ui_down
godot-qa input click qa:start_button
godot-qa input mouse click --x 500 --y 300
godot-qa input mouse wheel --x 500 --y 300 --dy 3
godot-qa input touch down --x 640 --y 360
godot-qa input touch move --x 400 --y 360 --frames 10
godot-qa input touch up
godot-qa input touch tap --x 640 --y 360
godot-qa input drag --from-x 600 --from-y 300 --to-x 200 --to-y 300
godot-qa input key Enter
godot-qa input type "127.0.0.1"
godot-qa input joy button start --device 2 --frames 2
godot-qa input joy motion right_x 1.0 --frames 2 --device 0
```

Action-level input is still preferred for stable menu tests, but the live session now also supports bounded physical input:

- `input mouse click --x --y`
  - left-button click at integer viewport coordinates
  - returns `device`, `button`, `position`, and target/resolved locator data when a visible control is hit
- `input mouse wheel --x --y [--dy <f>] [--dx <f>]`
  - emits mouse-wheel notches at viewport coordinates; see the S78 additive contract below for sign convention and payload
- `input touch down|move|up|tap [--x --y] [--index <n>] [--frames <n>]`
  - low-level screen-touch primitives for a single held finger per `--index`; see the S78 additive contract below for semantics and payloads
- `input drag --from-x --from-y --to-x --to-y [--frames <n>] [--pointer <touch|mouse>] [--index <n>]`
  - convenience one-shot drag from one point to another over `--frames` interpolated steps; see the S78 additive contract below for the `--pointer` enum and payloads
- `input key <key>`
  - one named key press/release, not freeform text entry
  - supported canonical keys: `Enter`, `Escape`, `Space`, `Tab`, `Backspace`, `Up`, `Down`, `Left`, `Right`, `0-9`, `A-Z`
  - aliases include `Return`, `Esc`, and `ArrowUp`/`ArrowDown`/`ArrowLeft`/`ArrowRight`
  - printable keys (`Space`, `0-9`, `A-Z`) now carry the matching `unicode` codepoint (space=32, digits 48-57, letters lowercase 97-122) so a focused `LineEdit`/text control inserts the character; control keys (`Enter`, `Escape`, `Tab`, `Backspace`, arrows) stay `0`
  - letters emit lowercase `unicode` with no Shift synthesis, so `input key A` inserts `a`; use `input type` for case-sensitive or punctuated text
  - success `data` now includes an additive `unicode` field (the emitted codepoint, or `0` for control keys)
- `input type <text>`
  - freeform text entry: injects a per-character `InputEventKey` press/release for each codepoint of `<text>`, with `unicode` set from the text's codepoints and a best-effort keycode (`OS.find_keycode_from_string` on the uppercased character, `KEY_NONE` fallback)
  - case-sensitive and supports punctuation and non-ASCII characters, unlike `input key`
  - no modifier (Shift/Ctrl/Alt) synthesis; each character is emitted directly by its `unicode`
  - empty text fails with `invalid_params`; length is bounded at `900` characters (2 frames per character against the `1800`-frame gesture cap)
  - the CLI derives the live-request timeout from the text length (2 frames per character at the same 30fps floor and grace used by `session.wait_frames`, capped at 120 seconds), so max-length text does not hit the flat 5-second request timeout
  - success `data` fields: `device`, `text`, `characters`, `focus_owner`
  - engine caveat: `input key Enter` submits a focused `LineEdit` and ends its editing state (default `keep_editing_on_text_submit`), so later typed keys are dropped until the field is re-activated — e.g. `input click qa:<field>` before typing again
- `input joy button <button> [--device <n>] [--frames <n>]`
  - one digital joypad-button press/release on the requested virtual device
  - `--device` defaults to `0`; `--frames` defaults to `1`
  - supported canonical buttons: `a`, `b`, `x`, `y`, `back`, `guide`, `start`, `left_shoulder`, `right_shoulder`, `left_stick`, `right_stick`, `dpad_up`, `dpad_down`, `dpad_left`, `dpad_right`
  - aliases include `select -> back` and `menu -> start`
- `input joy motion <axis> <value> [--device <n>] [--frames <n>]`
  - emits one joypad-axis motion event, waits the requested frame count, and leaves the axis value active until another motion command resets it
  - `--device` defaults to `0`; `--frames` defaults to `1`
  - supported canonical axes: `left_x`, `left_y`, `right_x`, `right_y`, `trigger_left`, `trigger_right`
  - aliases include `left_trigger -> trigger_left`, `right_trigger -> trigger_right`, `lt -> trigger_left`, `rt -> trigger_right`, and Godot constant-style names such as `joy_axis_right_x`
  - `value` must be numeric and within `-1.0..=1.0`; reset an axis with value `0.0`

Out of scope for S17: key chords/modifiers and multi-device joypad routing beyond explicit `--device`. Freeform text entry is now covered by `input type`/`input.text` (which supersedes this out-of-scope note for text entry); key chords/modifiers remain out of scope. Touch, drag, and wheel input are covered by the S78 additive contract below (it supersedes this out-of-scope note for those gestures).

## Touch, drag, and wheel input commands (S78 additive contract)

Touch, drag, and wheel commands are additive over the existing live `input` family and are available only against an active, token-protected live session. This section documents the mouse-wheel, touch, and drag commands.

Example one-shot command:

```bash
godot-qa input mouse wheel --x 500 --y 300 --dy 3 --json
```

Contracted options:

- `--x <i64>` and `--y <i64>` are required viewport coordinates. The addon accepts integer or float coordinate values and rejects coordinates outside the visible viewport with `invalid_params`.
- `--dy <f64>` and `--dx <f64>` are scroll deltas and default to `0.0`; at least one must be non-zero or the command fails with `invalid_params`.
- Sign convention is web-style: positive `--dy` scrolls down (`wheel_down`), negative scrolls up (`wheel_up`); positive `--dx` scrolls right (`wheel_right`), negative scrolls left (`wheel_left`).
- Each axis emits `max(1, ceil(|delta|))` press/release notch pairs with `factor = |delta| / pairs`, awaiting one frame per pair; the vertical axis is emitted before the horizontal axis.

Exit codes:

- `0` success (wheel notches emitted)
- `1` validation/runtime failure (including `invalid_params` for zero deltas or out-of-bounds coordinates)
- `2` CLI usage/parse failure

Success `data` fields:

- `device`
- `position`
- `dx`
- `dy`
- `buttons`

Structured failure payload fields:

- `type`
- `command`
- `expected`
- `actual`

### Touch commands

`input touch down|move|up|tap` inject low-level `InputEventScreenTouch`/`InputEventScreenDrag` events for one held finger per `--index`. They map to the WS-M gesture vocabulary: peek = `touch down` + `session wait`, drag = `touch move`, two-finger = distinct `--index` values, lift = `touch up`.

Example one-shot commands:

```bash
godot-qa input touch down --x 640 --y 360 --json
godot-qa input touch move --x 400 --y 360 --frames 10 --json
godot-qa input touch up --json
godot-qa input touch tap --x 640 --y 360 --json
# two-finger gesture: distinct --index fingers held at once
godot-qa input touch down --x 500 --y 300 --index 0 --json
godot-qa input touch down --x 780 --y 300 --index 1 --json
```

Contracted options:

- `--x <i64>` and `--y <i64>` are viewport coordinates. The addon accepts integer or float coordinate values and rejects coordinates outside the visible viewport with `invalid_params`. They are required for `down`, `move`, and `tap`; for `up` they are optional and default to the finger's last tracked position.
- `--index <i64>` selects the finger and defaults to `0`; distinct indices drive independent multi-touch fingers.
- `--frames <i64>` (only on `move`) defaults to `1` and interpolates the slide from the finger's last position to the target over that many frames (one `InputEventScreenDrag` per frame, the final step landing exactly on the target); it must not exceed `1800` (`invalid_params` otherwise).
- `move` and `up` require the finger to already be held; `down` and `tap` require the finger to be free.

Error kinds:

- `invalid_params` for bad coordinates, index, or frames (including a `move --frames` above `1800`).
- `touch_index_held` when `down`/`tap` targets a `--index` that is already held.
- `touch_index_not_held` when `move`/`up` targets a `--index` that is not currently held.

Held fingers left down are lifted automatically (normal `pressed=false` release, not a cancel) by the runtime's `input.reset` cleanup, which runs at scenario finalize and on live `session scene` changes. When `input_devices/pointing/emulate_mouse_from_touch` is enabled (Godot's default), injected touch also synthesizes mouse events; the runtime logs this once per process and never mutates the setting.

Success `data` fields:

- `touch down`: `device`, `index`, `position`, `pressed`
- `touch move`: `device`, `index`, `position`, `relative`, `frames`
- `touch up`: `device`, `index`, `position`, `pressed`
- `touch tap`: `device`, `index`, `position`, plus `target`/`resolved` when a visible control is hit

### Drag command

`input drag` is a convenience gesture that presses at `--from-x`/`--from-y`, interpolates to `--to-x`/`--to-y` over `--frames` steps (one event per frame, the last step landing exactly on the target), then releases. It owns its own press/release lifecycle, so it drives a fresh gesture rather than continuing an already-held finger.

Example one-shot commands:

```bash
godot-qa input drag --from-x 600 --from-y 300 --to-x 200 --to-y 300 --json
# left-button mouse drag instead of touch
godot-qa input drag --from-x 600 --from-y 300 --to-x 200 --to-y 300 --pointer mouse --json
```

Contracted options:

- `--from-x`/`--from-y`/`--to-x`/`--to-y` `<i64>` are required viewport coordinates for both endpoints. The addon accepts integer or float coordinate values and rejects either endpoint outside the visible viewport with `invalid_params` (the failing endpoint is named in `actual`).
- `--frames <i64>` defaults to `8` (a drag is inherently multi-frame) and must be at least `1` and no more than `1800` (`invalid_params` otherwise). The gesture interpolates over `max(frames, 1)` steps.
- `--pointer <touch|mouse>` selects the backing device and defaults to `touch`. The touch pointer emits `InputEventScreenTouch` down/up around `InputEventScreenDrag` steps; the mouse pointer emits a left-button press/release around `InputEventMouseMotion` steps that carry `button_mask = MOUSE_BUTTON_MASK_LEFT` so Control drag detection fires.
- `--index <i64>` defaults to `0` and applies only to the touch pointer (it selects the finger); it is ignored for the mouse pointer.

Error kinds:

- `invalid_params` for bad coordinates (either endpoint), `frames` below `1` or above `1800`, a bad `--index`, or a `pointer` outside `touch`/`mouse`.
- `touch_index_held` when the touch pointer targets an `--index` that is already held (drag owns its own down/up lifecycle).

Success `data` fields:

- `device` (`touch` or `mouse`), `pointer`, `index` (touch only — omitted for mouse), `from`, `to`, `frames`, `steps` (`steps == max(frames, 1)`).

## Assertion commands

```bash
godot-qa assert exists qa:pause_menu --scenario main_menu_start_opens_pause
godot-qa assert not_exists qa:loading_spinner --scenario main_menu_start_opens_pause
godot-qa assert text_equals qa:title "Settings" --scenario main_menu_start_opens_pause --snapshot after_start
godot-qa assert no_overlap qa:start_button qa:settings_button --scenario main_menu_start_opens_pause --run-id 20260421T120000Z
godot-qa assert no_errors --scenario main_menu_start_opens_pause
```

Current S6 behavior:

- snapshot-backed mode is implemented today and requires `--scenario`
- `--run-id` defaults to the latest artifact run directory for that scenario
- `--snapshot` defaults to the last snapshot named in `result.json`
- `--live` routes the normalized assertion over the active authenticated session transport
- `--live` cannot be combined with `--scenario`, `--run-id`, or `--snapshot`; those combinations return `type: "invalid_assertion_args"`
- live assertion failures preserve the same mismatch types and actionable fields (`target`, `expected`, `actual`, `severity`) as the runtime assertion layer
- target-resolving assertions use the shared Target locators contract, including fast `qa.id.<id>` group lookup, `qa.<id>` compatibility groups, `qa_id` and `qa_aliases` fallback scans, deduplicated ambiguity lists, and optional rejected-candidate diagnostics on missing `qa:` targets

## Layout commands

```bash
godot-qa layout audit-current --scenario settings_menu_layout --run-id 20260421T120000Z --snapshot default
godot-qa layout audit-current --scenario sample_settings_layout --live
godot-qa layout audit-current --live
godot-qa layout audit --scene res://ui/settings_menu.tscn --viewports 1280x720,1920x1080,2560x1440
```

Current S20 contract:

- `layout audit-current` requires `--scenario` unless `--live` is set; omitting both returns `type: "invalid_layout_args"`
- with `--live`, `--scenario` is optional: a live session already knows where its artifacts live, so `--scenario` only labels the captured snapshot and defaults to `current` when omitted
- `--run-id` defaults to the latest artifact run directory for that scenario
- `--snapshot` defaults to the last snapshot named in `result.json`
- `--live` captures a fresh snapshot bundle from the active authenticated session, then feeds those `tree`/`ui`/`focus`/`errors` artifacts into the same layout engine used for saved runs
- `--live` returns `data.live: true` plus the staged live snapshot artifact paths instead of a synthetic `run_id`
- `--live` cannot be combined with `--run-id` or `--snapshot`; those combinations return `type: "invalid_layout_args"`
- `--strict` upgrades warning-level layout findings to failing severity for the returned result
- `layout audit --scene ... --viewports ...` is the dedicated viewport sweep command; it captures one run per requested viewport and returns aggregated failures

A `scenario run` (or `scenario profile`) with a `layout_audit` assertion is routed by viewport count. **Zero or one `viewports` entry runs the NORMAL single-run path** and folds the layout audit over the run's final snapshot as a CLI post-process — the result is the ordinary single-run envelope (`data.snapshots`, `data.outcome`, layout failures inline), *not* a matrix envelope. A lone `viewports` entry still sets the window resolution, exactly as elsewhere. Because it is a post-process, `layout_audit` and `screenshot_match` now coexist on a single-viewport run. **Two or more `viewports` entries fan out into the viewport matrix** and keep the aggregate `data.viewports` + `data.runs[]` envelope, auditing each cell's last snapshot. Either path, when the run records no snapshot to audit, fails with `layout_audit_missing_snapshot` (add a `snapshot` step) instead of passing vacuously. A viewport matrix that somehow resolves to zero runs fails with `viewport_matrix_empty`. (Combining `screenshot_match` with a multi-entry `viewports` matrix is rejected statically as `scenario_validation_failed` — the lint-time mirror of the runtime `screenshot_match_viewports_unsupported` guard.)

See `docs/layout-audit-rules.md`.

## Live edit commands

```bash
godot-qa edit-live set qa:menu_panel custom_minimum_size "900,600"
godot-qa edit-live call qa:start_button grab_focus
godot-qa edit-live diff
godot-qa edit-live export-patch --out qa/patches/settings_menu_patch.json
godot-qa apply-patch qa/patches/settings_menu_patch.json
```

Current S10 behavior:

- `edit-live set` routes through the authenticated live session transport, resolves the target in Godot, coerces the string input to the live property type in Godot, applies the mutation, and returns the recorded ledger entry.
- `edit-live call` currently supports the narrow allowlist `grab_focus` and records the call as live-only.
- `edit-live diff` returns the append-only edit ledger for the active session.
- `edit-live export-patch` writes only exportable `set_property` operations to the requested JSON file, stamps the patch with `created_at` plus `created_from.session_id/session_meta`, and also returns `skipped_operations` for live-only edits.
- `apply-patch` validates the patch JSON on the CLI side, prepares artifacts under `.qa/artifacts/patch-apply/<patch-stem>/<run-id>/`, then launches `qa_patch_runner.gd` to validate and save the scene explicitly.

Current property allowlist:

- `CanvasItem`: `visible`, `modulate`, `self_modulate`
- `Control`: `custom_minimum_size`, `offset_left`, `offset_top`, `offset_right`, `offset_bottom`, `focus_mode`
- `Label`: `text`
- `BaseButton`: `text`, `disabled`

Current typed value support for patch export/apply:

- JSON scalars: `String`, `bool`, `int`, `float`, `null`
- typed envelopes: `Vector2`, `Vector2i`, `Color`

Live edits are temporary unless exported and applied as source patches.

## Scenario bundle commands

```bash
godot-qa scenario bundle list --json
godot-qa scenario bundle run <name> [<name>...] [--jobs <signed-count>] [--strict] [--headless|--windowed] [--retry-startup <count>] [--retry-startup-backoff-ms <ms>] [--only <scenario>]... [--failed-only] [--schedule manifest|longest-first] --json
```

Base usage: `godot-qa scenario bundle run <name> [--jobs <signed-count>] [--strict] [--headless|--windowed] --json`.

Bundles are declarative regression-gate manifests loaded from `qa/gates/` and are not a CI scripting language.
They may compose other bundles with `required_bundles`; expansion is bounded, detects cycles before any child runs, and rejects duplicate required scenarios with source-bundle metadata.
`scenario bundle run` executes child scenarios through the same direct `scenario run <name>` behavior path; it does not replace or fork per-scenario runtime behavior.
Child scenario `godot_args` and `project_args` are preserved exactly as direct scenario runs preserve them. Bundle summaries include additive source fields rather than replacing existing result fields, and persisted summary metadata redacts secret-looking `project_args` values while preserving count and order.
`scenario lint` remains a separate command and is not auto-invoked by bundle run unless a future explicit option is added.

Strict pass/fail semantics are unchanged. Bundle runs do not retry by default, and a child failure still fails the bundle unless the existing optional-scenario or strict-mode rules say otherwise. Startup retry is opt-in, bounded by `--retry-startup <count>`, delayed by the documented `--retry-startup-backoff-ms <ms>` value when provided, and visible in JSON. Retry only applies to infrastructure-only startup failures before gameplay assertions begin: `display_startup_exhausted`, `virtual_display_allocation_failed`, `stale_process_cleanup_failed`, and `scenario_result_missing` when the missing child `result.json` follows a display startup failure. Retry never applies to gameplay/assertion failures, script errors, malformed scenario definitions, malformed artifacts, malformed `result.json`, screenshot quality failures, visual assertion failures, log health failures, crashes after scenario execution begins, or normal `godot_run_failed` outcomes.

Bundle children run in independent Godot processes with bounded process-level parallelism. Omitted `--jobs` falls back to the `qa/config.yaml` `scenario.jobs` project default when set, else to `0`; `0` resolves to `std::thread::available_parallelism()` with a fallback of `1`; a positive value requests that many workers; and a negative value reserves that many available threads, for example `--jobs -2` on an eight-thread host resolves to six workers. The bundle result reports where the effective value came from in `data.jobs_origin` (`"flag"`, `"config"`, or `"default"`). The effective worker count is capped to the number of runnable child scenarios. All runnable children are queued even when another required child fails; missing required scenarios and scenario load failures still fail the bundle but do not block other resolvable children. `data.runs[]` remains ordered by bundle expansion index rather than completion order.

**Selecting children.** `--only <scenario>` (repeatable) restricts the run to the named children of the bundle, and `--failed-only` restricts it to the children that did not pass in that bundle's most recent run. Both are matched against the bundle's expanded scenario names, and together they intersect. Every run reports `data.selection` — `mode` (`"all"` or `"filtered"`), `only[]`, `failed_only`, `source_run_root` (the run directory a `--failed-only` baseline was read from, else `null`), `member_count`, and `selected_ids[]` — in the command JSON, in `summary.json`, and in `meta.json`, so the key is always present rather than implying "unfiltered" by its absence. A filtered run appends `(filtered: k of n)` to the result message; an unfiltered run's message is unchanged. A deselected scenario is dropped before it is looked up or loaded, so it contributes no `missing_scenario` failure, no preparation warning, and no `--strict`/`--warm-strict` violation: narrowing a run never changes the verdict of a scenario it did not run. `data.runs[].index` stays the bundle expansion index, and the warm below-parallelism skip is decided on the filtered count, so `--warm --jobs 2 --only a --only b` skips warm exactly as a two-scenario bundle would.

A selection that cannot be resolved fails the invocation *before* anything runs, with exit code 2, an empty `data`, and no run directory written: `selection_unknown_scenario` when `--only` names a scenario the bundle does not contain (`expected` lists the members, `actual` the unknown names) and `selection_no_previous_run` when `--failed-only` finds no previous run, or one whose `summary.json` cannot be read (`actual` is then `"unreadable: <error>"`). An *empty* selection is not an error — the previous run may simply have been green — so the run proceeds, executes nothing, exits 0, and adds a `selection_empty` warning to `data.warnings[]` and to `summary.json`. `--failed-only` always reads the newest run's `summary.json` regardless of what that run itself selected, so repeated `--failed-only` invocations narrow monotonically as failures are fixed, and one unfiltered run resets the baseline. Names an old summary carries that the bundle no longer contains are ignored.

**Dispatch order.** `--schedule <manifest|longest-first>` chooses the order children are handed to the workers. Omitted, it falls back to the `qa/config.yaml` `scenario.schedule` project default, else to `longest-first`. Under `longest-first` and more than one worker, the bundle reads the duration of each scenario from the newest five readable `summary.json` files under `.qa/artifacts/bundles/<bundle>/` (newest non-null value per scenario wins) and dispatches the slowest-predicted children first, so the longest scenario cannot land in the parallel tail. Scenarios with no recorded duration sort *first*, because an unmeasured cost is unbounded; the first-ever run of a bundle is therefore plain expansion order. `manifest`, a serial run (`--jobs 1`), and a bundle with no usable history leave the order alone and skip the history read entirely. Every run reports `data.scheduling` — `policy`, `origin` (`"flag"`, `"config"`, or `"default"`), `applied`, `history_runs`, `predicted`, and `unknown` — in the command JSON and in `summary.json`; `history_runs`, `predicted`, and `unknown` are `0` whenever `applied` is false. Dispatch order is a scheduling detail only: it never changes which children run, their `index`, or the reported ordering of `data.runs[]`.

**Running several bundles.** `scenario bundle run <name> [<name>...]` runs each named bundle in turn under ONE envelope. This is not three invocations glued together: the run-level resources are created once and shared — with `--warm` that means a single warm session pool for every bundle, so a small gate that would skip warm on its own count runs on the pool the other gates already paid for (see the warm section below). Every name is resolved before anything runs, so a typo in the third gate costs no work and every unresolvable name is reported at once. Each bundle still writes exactly the artifacts a solo invocation writes, keeps its own verdict, and records the full multi-name command in its `meta.json`.

The multi-bundle envelope reports `data.bundles[]` — one entry per name, in the order given, carrying that bundle's own `data` plus `name`, `ok`, and its `failures[]` — and `data.summary` (`total`, `passed`, `failed`). The message is `scenario bundle run completed (N bundles)` or `scenario bundle run failed (K of N bundles failed)`, and the exit code is `1` if any bundle failed. Top-level `failures[]` is every bundle's failures concatenated, each stamped with the `bundle` it came from, so the usual "read `failures[0]`" reflex still identifies what broke. `data.timing` carries the invocation's `wall_ms` plus `pool_seed_ms` / `pool_shutdown_ms` for the shared pool (`null` without one). Anything describing a resource the *invocation* owns is reported once, at the envelope level, and never claimed by a bundle: `data.warm_pool` (the pool counters) is absent from every `data.bundles[]` entry and from every bundle's `summary.json`, because no bundle started or stopped those sessions; the envelope's `data.warm` is the one warm decision, while a bundle's own `data.warm` says only whether it ran warm; the envelope's `data.retention` reports the single pooled-session prune, while each bundle's `data.retention` reports its own scenario and bundle run prune with `pruned.sessions: 0`; and pool-level warnings appear once in the envelope's `data.warnings[]` instead of being repeated under each bundle. A single name is not "a multi-bundle run of one" — its envelope is unchanged, with no `bundles` key.

Two argument shapes are rejected before anything is loaded, launched, or written, with exit code 2, an empty `data`, and a failure of type `invalid_arguments`: the same bundle named more than once, and `--equivalence-check`, `--only`, or `--failed-only` given alongside more than one name. Those three are per-bundle by definition — `--only`/`--failed-only` select children of one bundle and `--equivalence-check` needs a pool it owns alone — and there is no syntax yet for saying which bundle they apply to, so they are refused rather than silently applied to all of them. A `--failed-only` that spans several bundles is the natural follow-up.

When the project sets `artifacts.keep_runs`, `scenario bundle run` reports the prune under `data.retention`: `keep_runs`, `pruned` (`scenario_runs`, `bundle_runs`, `sessions` counts), and `errors`. `scenario run` reports the same object with `pruned.scenario_runs` only. The key is absent entirely when the project has not set `artifacts.keep_runs`. Prune errors add an `artifact_prune_failed` warning to `data.warnings[]` and never change `ok`, `failures`, or the exit code. The warning is command-JSON only: pruning runs after the bundle has written `summary.json`, so that file never records it. Every bundle run also rotates `.qa/artifacts/bundles/<bundle>/latest`, a relative symlink on Unix or an NTFS directory junction on Windows naming the run directory just written, and reports it as `data.artifacts.latest_path`. The link is a convenience alias, not evidence: retention never prunes it and never leaves it dangling (it always names the newest run, which is always retained), Windows junctions need no Developer Mode or administrator symlink privileges, and a rotation failure never changes the verdict. Junction targets are absolute; recreate convenience aliases after moving artifact directories.

Bundle-level failures include `category` so agents can distinguish `infrastructure`, `gameplay_assertion`, `process`, `display`, `artifact`, and `log_health` outcomes without changing existing failure `type` values. Infrastructure startup failures include `retry_guidance` with whether retry was eligible, attempted, exhausted, and which command/flags can reproduce or adjust the bounded retry.

Every bundle run reports a wall-clock account of itself in `data.timing`, so "which scenario is slow" is answerable from the bundle JSON instead of by opening one child `result.json` per scenario. It holds `started_at_ms` and `finished_at_ms` (epoch milliseconds), `wall_ms`, `prepare_ms` (bundle discovery, expansion, and child preparation), `execute_ms` (every child run), `artifacts_ms` (writing the bundle artifacts), `sum_child_duration_ms` (the sum of the children's engine-reported `duration_ms`), `parallel_efficiency` (`sum_child_duration_ms / (execute_ms × resolved_jobs)`: the share of held worker time actually spent inside Godot — advisory, deliberately unclamped, and `null` when nothing ran), `slowest` (up to ten children by `duration_ms`, worst first, each with `scenario`, `duration_ms`, `scene_load_ms`, `steps_ms`, and `run_root`), and `pool_seed_ms` / `pool_shutdown_ms` (the warm pool's seed boot and shutdown when the run had a pool, `null` otherwise). Each `data.runs[]` entry gains the same accounting per child: `duration_ms` and `timing` copied verbatim from that child's `result.json`, `started_at_ms` / `finished_at_ms` (CLI wall clock around the child, spanning any bounded startup retry), `lane` (which worker ran it, always `0` on the serial path), and `queue_wait_ms` (how long it waited for a free lane). `data.runs[].attempts[]` carries `duration_ms`, `timing`, `started_at_ms`, and `finished_at_ms` per attempt. Every span is milliseconds rounded to microseconds; the whole block is advisory metadata and never changes a verdict or an exit code. `summary.json` carries the same block as `timing`, but as a snapshot taken *before* that file was written: its `artifacts_ms` is `0` and its `wall_ms` / `finished_at_ms` exclude the summary write itself, while the command JSON has the final numbers.

Every failure raised by a child scenario carries `scenario` (the run it came from) and `run_root` alongside its existing fields, in the command JSON `failures[]` and in `summary.json`, so a flat failure list is attributable without cross-referencing `data.runs[]`. Failures raised while *preparing* the bundle (`missing_scenario`, `duplicate_required_scenario`, and friends) are untouched — they already name their manifest and have no run to point at. Repeated child failures are additionally indexed in `data.failure_clusters[]` (and in `summary.json`): failures group by `type` plus the failure's `provider` when it names one, else its `target`, and any group spanning at least three distinct scenarios is reported as `{kind, key, scenario_count, failure_count, scenarios, hint}`, where `scenarios` lists the first ten and `hint` names the likely shared cause — for `provider_not_registered`, that the provider is unregistered in N of M scenarios and that the registration should be fixed before reading per-scenario failures. The key is always present, as an empty array when nothing clusters. Clustering is purely additive: no failure is suppressed, reordered, or rewritten, and the verdict is unchanged.

Bundle runs write `artifact-index.json` next to `summary.json`. The standard JSON envelope includes the concise child summary as `data.artifact_summary`, and `data.artifacts.artifact_index_path` records the written artifact path. The file is a concise index of child scenario evidence with `scenario`, `status`, `run_root`, `result_path`, `logs_path`, `screenshot_paths`, `diagnostic_paths`, `failure_artifact_paths`, `source_bundle`, and `bundle_chain`. It references child artifacts without replacing child `result.json`, logs, screenshots, diagnostics, or detailed failure artifacts.

### Warm bundle runs and `--warm-strict`

```bash
godot-qa scenario bundle run <name> --warm [--warm-strict] [--equivalence-check] --json
godot-qa scenario bundle run <name> <name> [<name>...] --warm [--warm-strict] --json
godot-qa scenario bundle run <name> --cold [--no-warm-strict] --json
```

`--warm` reuses a shared Godot process across every scenario in the bundle instead of booting a fresh process per scenario (a much faster dev loop). Warm scenarios split into three buckets:

- **headless-warm** — no windowed-media need. Runs in a shared HEADLESS warm session (the historical warm behavior).
- **windowed-warm** — the *only* reason it would need cold is windowed media evidence (a saved screenshot, a visual pixel assertion such as `not_occluded`/`saved_snapshot_pixels`/`renders_above`/`renders_below`/`visible_pixels`, a `screenshot_match` assertion, a single-viewport `layout_audit` — which reads the run's UI snapshot + screenshot evidence — or audio capture), AND it is single-viewport, its `determinism.fixed_fps` matches the session, it has no per-scenario `godot_args`, and it is not declared `execution.isolation: cold`. Runs in a shared WINDOWED warm session so it captures real pixels between scene reloads instead of relaunching Godot per scenario.
- **truly-cold** — declared `execution.isolation: cold`, per-scenario `godot_args`, more than one viewport (`viewports.len() > 1`, which subsumes a multi-viewport `layout_audit` matrix), or a `determinism.fixed_fps` that differs from the warm session's. Falls back to the authoritative cold path (a fresh process) per scenario. A structural cold reason takes precedence over a media reason: a scenario that needs windowed media AND (say) its own `godot_args` is truly-cold. A *single*-viewport `layout_audit` is NOT structural — it runs the normal single path and is windowed-warm.

Warm scales with `--jobs` exactly like cold: a pool of up to K independent warm sessions (K = the resolved parallelism, `--jobs 1` collapsing it to a single serial session) runs headless-warm scenarios concurrently, each session reusing its Godot process across the scenarios routed to it. Windowed-media scenarios share one serialized windowed session (concurrent windowed Godot is display-risky and windowed-media scenarios are rare). Any scenario whose warm round-trip itself fails (a busy/dead session, a transport error) falls back to a fresh cold process, and if the initial session cannot start the whole bundle runs cold — so `--warm` never produces a worse outcome than a plain bundle run.

**A failed session start is bounded, not terminal.** Each pooled lane tolerates two *consecutive* failed starts; any successful start resets the count. Below that budget the lane keeps booting, so one transient failure — a lost race on a shared artifact path, a port that was momentarily unavailable — costs a single scenario a cold run instead of condemning the lane for the rest of the bundle. When a lane is at capacity, or has spent its budget but still holds live sessions, a worker *waits* for a peer to check one back in instead of immediately paying a cold boot. That wait is bounded by the seed session's measured boot time, clamped to 250 ms – 5 s, because waiting longer than one boot can never beat just running cold. Only when the deadline passes, or the lane holds neither a live session nor any remaining budget, does the scenario fall back to cold with `warm_session_unavailable`. A session retired after a failed round-trip (`warm_transport_failure`) frees its slot *without* counting against the boot budget, so one wedged session never blocks its own replacement.

**Every pool failure is reported.** A `--warm` bundle result carries `data.warm_pool` with `sessions_started`, `session_start_failures` (each entry is the full startup failure plus the `lane` — `headless_dummy`, `headless_real`, or `windowed` — and the `attempt` number within that checkout), `transport_failures` (each with the `scenario` and `session_id` whose round-trip failed), `seed_boot_ms`, `shutdown_ms`, and `acquire_wait_ms_budget`. `sessions_started`, `session_start_failures`, and `transport_failures` are mirrored onto `data.warm` for consumers that read only that object. Each failed session start additionally becomes a bundle-level **warning** — in the command JSON `warnings` and therefore in `summary.json` too — with `type: "warm_session_start_failed"`, `command: "scenario bundle run"`, `path` set to the session artifact directory when one existed, `actual` set to the original failure kind, and the whole startup failure under `cause`. These warnings are infrastructure evidence: bundle `--strict` never promotes warnings to failures, and `--warm-strict` stays exempt from session-start failures exactly as described below. Pooled sessions are also stopped in parallel at the end of the run, before the bundle summary is written, so `shutdown_ms` is inside the bundle's own accounting rather than invisible wall clock after it. When several bundles are named in one invocation the pool outlives each of them, so the shutdown happens once after the last bundle instead: no bundle's `summary.json` reports `pool_seed_ms`, `pool_shutdown_ms`, or `warm_pool` (it neither started nor stopped those sessions), and the envelope's `data.timing` and `data.warm_pool` carry them once for the invocation.

**Warm is skipped entirely when the bundle has no more scenarios than parallel lanes** — that is, when resolved parallelism ≥ the runnable scenario count. In that regime every scenario would get its own fresh session (one Godot boot each, exactly like cold), so no session is ever reused across scenarios and warm would only add per-scenario bridge overhead on top of the same boot cost. The whole bundle runs on the already-parallel cold path instead, and the bundle result carries `data.warm = {"enabled": true, "session_started": false, "ran_warm": false, "mode": null, "reason": "no_reuse_below_parallelism"}`. Warm therefore helps only when a bundle has many more scenarios than cores, where each pooled session amortizes its one boot across several scenarios. `--jobs 1` with more than one scenario keeps warm (a single session reused serially, so there *is* reuse); a single-scenario bundle always runs cold. `--warm --equivalence-check` is never skipped this way (it needs the warm leg to compare against cold).

**Several bundles named in one `--warm` invocation share ONE pool, and therefore one skip decision.** The pool is seeded once before the first bundle and shut down once after the last, and the skip is decided on the SUM of the bundles' runnable scenario counts rather than per bundle — three six-scenario gates on a twelve-lane host are eighteen scenarios over twelve lanes, which does reuse sessions, even though each gate alone (6 ≤ 12) would have run cold. When the invocation does skip, `data.warm` carries `reason: "no_reuse_below_parallelism"` plus `runnable_count_total`, the summed count that decided it, and every child is annotated `warm_skipped_below_parallelism` exactly as in a single skipped bundle. A bundle whose scenario count cannot be resolved makes the sum unknowable, and an unknown sum never justifies skipping: the normal path runs and reports that bundle's real failure. Under `--warm-strict` an initial seed failure still fails the whole invocation immediately; without strict, every bundle runs cold and the cause is attached to the envelope's `data.warm` once rather than repeated under each bundle.

The parallelism skip does **not** let `--warm-strict` off the hook. Every strict-relevant warm violation is *structural* — computable from the scenario document alone (the truly-cold reasons above: `godot_args`, `multi_viewport`, `fixed_fps_mismatch`, `frame_pacing_invalid`), needing no warm session — so during a skipped `--warm-strict` run each scenario is still classified and any structural cold fallback is appended as the same `warm_fallback_forced_cold` failure a real warm session would produce (a declared `execution.isolation: cold` opt-out stays exempt, and a warm-eligible scenario that merely runs cold here is not a violation). The bundle still runs the parallel cold path — jobs are *not* forced to 1 and nothing hard-errors. The result is that `--warm-strict` gives the **same verdict on any host core count**: a 12-core host that skips warm and a 1-job CI reach the identical pass/fail. When strict is enforced during a skip, `data.warm` additionally carries `"strict_structural_check": true` so artifacts show strict was applied despite the skip; without `--warm-strict` the field is absent and the skip is a plain cold run.

Every scenario run under `--warm` records `data.warm` with whether it actually ran warm and in which mode:

```json
{"warm": {"ran_warm": true, "declared": false, "mode": "windowed", "reason": null}}
```

or, when it fell back to cold:

```json
{"warm": {"ran_warm": false, "declared": false, "mode": null, "reason": "godot_args"}}
```

`mode` is `"headless"` or `"windowed"` when the scenario ran warm, else `null`. `reason` is one of `execution_isolation_cold`, `godot_args`, `screenshot_match`, `windowed_media` (the media reason for a windowed-warm scenario, including a single-viewport `layout_audit`), `multi_viewport`, `fixed_fps_mismatch`, `frame_pacing_invalid`, `warm_transport_failure` (the warm session round-trip failed mid-bundle), `windowed_session_unavailable` (the windowed warm session could not start, so a windowed-warm scenario ran cold), or `warm_session_unavailable` (no pooled headless session could be acquired before the wait deadline — start failures exhausted the budget, or every live session stayed checked out). A bundle that skipped warm below parallelism adds one more reason, `warm_skipped_below_parallelism`: a scenario that would have run warm on a smaller host ran cold instead, while a truly-cold scenario in the same skipped bundle still reports its own reason. `declared` is `true` only for `execution_isolation_cold` — an intentional, author-declared opt-out. Bundle summaries (`data.runs[].warm`) surface the same per-scenario status, and each scenario's persisted `result.json` carries the same `data.warm` annotation (plus any strict failure) so the on-disk artifact matches the summary.

A scenario that actually ran warm additionally carries `data.warm.timing` — advisory wall-clock spans (ms floats) attributing where the warm run's time went on the CLI side: `artifact_prep_ms` (request/metadata writes before dispatch), `dispatch_ms` (the blocking `scenario.run` round-trip, which contains all engine work), `engine_ms` (the engine's own in-scenario wall clock, mirroring `duration_ms`), `finalize_ms` (result read-back, validation, and compares), `lane_held_ms` (how long the pooled session was checked out for this scenario), and `acquire_wait_ms` (how long the scenario waited to check a pooled session out, including any lazy boot it had to perform). These spans are in-memory bundle metadata for performance attribution; they are advisory, never part of equivalence comparison, and their exact set may evolve.

Bundle discovery and normalized scenario documents are prepared once per
invocation and shared by warm eligibility and execution. Bundle timing adds
`dispatch_wait_ms` for pool startup or earlier-bundle execution between
preparation and dispatch. The actual preparation start anchors `started_at_ms`
and `wall_ms`; multi-bundle wall spans can overlap. No persistent discovery cache
is used, and scenario selection, strictness, and result ordering are unchanged.

A scenario that actually ran warm also gets the SAME engine-log reporting a cold run gets. A pooled warm session appends every scenario's engine output to one long-lived `logs.txt` under `.qa/artifacts/live/<session-id>/`, so the addon brackets each warm scenario with `[godot-qa] scenario-log begin|end <run-id>` marker lines on stderr and the CLI materializes that window as the run's own `<run_root>/logs.txt` before finalizing. `data.logs` (`status`, `error_count`, `warning_count`) and the `godot_log_error` / `godot_log_warning` failures are therefore produced by exactly the same code as cold, from exactly the same ignore list; each such failure additionally carries `warm_log_window` (`session_id`, `begin_offset`, `end_offset`) pointing at the session-log bytes it was read from. Every warm run records `data.warm.session_id` and `data.warm.log_slice` (`source`, `session_id`, `status`, `begin_offset`, `end_offset`), where `status` is `sliced` (window found and written), `markers_missing` (the project's addon predates the markers — no `logs.txt` is written, `data.logs.status` stays `"missing"` exactly as before, and the bundle gains one `warm_log_markers_missing` warning), or `unavailable` (the session had no readable host-side log). Engine output emitted outside a window — after a scenario's `end` marker and before the next scenario's `begin` — is unattributed and stays only in the session log, which the CLI never truncates, rotates or reopens. Those gap lines are not lost, though: after the pool shuts down the bundle scans every session log it started for `ERROR:`/`WARNING:` lines outside all windows — through the same ignore list the cold scan uses, plus the session's own `resources still in use at exit` shutdown noise — and reports them as `data.warm.session_log_noise` (`sessions_scanned`, `line_count`, `by_message[]` of `line`/`count`/`sessions`, and `phase: "outside_scenario_windows"`). When `line_count` is above zero the bundle also gains ONE `warm_session_log_noise` warning. This is advisory evidence about session start, idle time and shutdown, never a failure: no scenario can be blamed for it, so neither `--strict` nor `--warm-strict` promotes it. That shared ignore list includes the project's own `logs.ignore` patterns: one list configured once in `qa/config.yaml` is applied identically by cold runs, warm runs, and the warm session-noise scan, so a pattern never has to be configured twice and a line silenced for cold can never reappear as warm-only noise. Each run reports what the list did through `data.logs.ignored_count` and `data.logs.ignore_patterns_applied`. Because there is no warm special-casing anywhere downstream, an engine error that appears only under warm is a genuine `--equivalence-check` divergence, not a tolerated difference.

`--warm-strict` (requires `--warm`, either via the flag or a `scenario.warm: true` project config default) turns a *strict-violating* fallback into a hard scenario failure (`warm_fallback_forced_cold`), failing the bundle the same way any other child failure does. This exists so an agent that unintentionally adds something incompatible with warm execution (or a warm session that silently degrades) is caught immediately instead of just quietly costing bundle-run speed. The strict-violating reasons are the structural ones — `godot_args`, `multi_viewport`, `fixed_fps_mismatch`, `frame_pacing_invalid` — plus `warm_transport_failure`. The following are **exempt** (they run — warm or cold — without a violation): a media-only scenario that runs windowed-warm (it is `ran_warm: true, mode: windowed`, no longer a fallback); a declared `execution.isolation: cold` opt-out; and a session-start failure (`windowed_session_unavailable` / `warm_session_unavailable`), which is an infrastructure condition, not a scenario declaration. Also exempt is `warm_skipped_below_parallelism`, which describes the host's core count rather than anything about the scenario.

Because every strict-violating reason except `warm_transport_failure` is structural (computable from the scenario document with no warm session), `--warm-strict` gives a **host-independent verdict**: even when warm is skipped below parallelism (see above) strict is still enforced by classifying each scenario structurally, so a bundle passes or fails strict identically regardless of `--jobs` or host core count — there is no host on which an incompatible scenario silently slips through.

Under `--warm-strict`, the *initial* (headless) warm session failing to start fails the bundle immediately with `warm_session_start_failed` — no cold bundle runs first, because paying for an all-cold bundle is exactly what strict mode guards against. But a *windowed* warm session that fails to start mid-bundle does not fail strict: its windowed-warm scenarios fall back to cold with reason `windowed_session_unavailable`, mirroring how a session-start failure is never a scenario's fault. Without strict, a failed initial session start keeps the previous behavior: the whole bundle runs cold (`data.warm.mode: "session_start_failed"`) so `--warm` is never worse than a plain bundle run — but it no longer degrades silently, because `data.warm.start_failure` carries the startup failure that caused it. Under strict the same failure is attached to the `warm_session_start_failed` failure as `cause`.

`--warm-strict` composes with `--equivalence-check`: the cold result stays the authoritative gate and the equivalence comparison is unaffected (the warm leg is compared before strict is applied), but a strict-violating fallback still adds `warm_fallback_forced_cold` to the gate result and fails the bundle. Windowed-warm scenarios run their warm leg in the shared windowed session (still cold as the authoritative leg). Truly-cold scenarios skip the warm leg entirely during an equivalence run — there is nothing to compare, so they no longer pay for a redundant second cold process.

`--cold` forces a plain cold run outright for this invocation, overriding a `scenario.warm`/`scenario.warm_strict` project config default unconditionally — the escape hatch for a one-off cold run on a project that defaults to warm. `--no-warm-strict` disables strict mode for this invocation the same way, without needing `--cold` too. `--warm-strict` without `--warm` (and without a `scenario.warm: true` config default) fails fast with `warm_strict_requires_warm` before anything runs.

A project can set defaults in `qa/config.yaml` so these flags don't need to be repeated on every invocation:

```yaml
scenario:
  warm: true
  warm_strict: true
  jobs: 1
```

`scenario.jobs` pins the default bundle parallelism for the project (same signed semantics as `--jobs`; an explicit `--jobs` flag overrides it). Useful when the host or project makes the available-parallelism default pathological — for example a windowed lane starved by many concurrent headless lanes.

CLI flags always take precedence over these config defaults. Projects whose scenarios rely on process-global state that can't be reset between scenarios should leave `scenario.warm` unset (or `false`) — the default remains a plain cold bundle run, unchanged.

Sample composed gate commands:

```bash
godot-qa scenario bundle list --project examples/sample_project --json
godot-qa scenario bundle run composed_regression_gate --project examples/sample_project --json
godot-qa scenario bundle run screenshot_heavy_regression_gate --project examples/sample_project --json
```

Example bundle-list success payload:

```json
{
  "ok": true,
  "command": "scenario bundle list",
  "message": "found 2 bundle(s)",
  "data": {
    "bundles": [
      {
        "name": "composed_regression_gate",
        "path": "/path/to/project/qa/gates/composed_regression_gate.yaml",
        "required_bundles": ["reusable_menu_gate"]
      }
    ]
  },
  "failures": []
}
```

Bundle run `data.runs[]` and the written `summary.json` include additive expansion metadata for each child:

```json
{
  "scenario": "bundle_godot_args_demo",
  "source_bundle": "composed_regression_gate",
  "bundle_chain": ["composed_regression_gate"],
  "godot_args": ["--single-window"],
  "project_args": ["--save-slot", "qa-profile"],
  "launch_display": {"mode": "headless"},
  "run_root": "/path/to/project/.qa/artifacts/scenarios/bundle_godot_args_demo/20260502T120000Z",
  "status": "passed"
}
```

Example bundle-run success artifact fields:

```json
{
  "ok": true,
  "command": "scenario bundle run",
  "message": "scenario bundle run passed",
  "data": {
    "bundle": "sample_regression_gate",
    "parallelism": {
      "raw_jobs": 0,
      "resolved_jobs": 4,
      "source": "available_parallelism",
      "available_parallelism": 4,
      "reserved_count": 0,
      "runnable_count": 4,
      "complete_queued": true
    },
    "jobs_origin": "default",
    "artifact_summary": {
      "children": []
    },
    "artifacts": {
      "run_root": "/path/to/project/.qa/artifacts/bundles/sample_regression_gate/20260502T120000Z",
      "meta_path": "/path/to/project/.qa/artifacts/bundles/sample_regression_gate/20260502T120000Z/meta.json",
      "summary_path": "/path/to/project/.qa/artifacts/bundles/sample_regression_gate/20260502T120000Z/summary.json",
      "artifact_index_path": "/path/to/project/.qa/artifacts/bundles/sample_regression_gate/20260502T120000Z/artifact-index.json",
      "result_path": "/path/to/project/.qa/artifacts/bundles/sample_regression_gate/20260502T120000Z/result.json"
    }
  },
  "failures": []
}
```

Example `artifact-index.json` entry:

```json
{
  "bundle": "composed_regression_gate",
  "entries": [
    {
      "scenario": "bundle_godot_args_demo",
      "status": "passed",
      "run_root": "/path/to/project/.qa/artifacts/scenarios/bundle_godot_args_demo/20260502T120000Z",
      "result_path": "/path/to/project/.qa/artifacts/scenarios/bundle_godot_args_demo/20260502T120000Z/result.json",
      "logs_path": "/path/to/project/.qa/artifacts/scenarios/bundle_godot_args_demo/20260502T120000Z/logs.txt",
      "screenshot_paths": [],
      "diagnostic_paths": [],
      "failure_artifact_paths": [],
      "source_bundle": "composed_regression_gate",
      "bundle_chain": ["composed_regression_gate"]
    }
  ],
  "warnings": [
    {
      "type": "missing_optional_scenario",
      "severity": "warning",
      "scenario": "optional_demo",
      "source_bundle": "composed_regression_gate",
      "bundle_chain": ["composed_regression_gate"]
    }
  ]
}
```

For display-isolated bundle runs, each child entry may include attempt metadata while preserving the existing child run roots and run-level artifact paths:

```json
{
  "scenario": "visual_occlusion_demo",
  "source_bundle": "screenshot_heavy_regression_gate",
  "bundle_chain": ["screenshot_heavy_regression_gate"],
  "status": "failed",
  "category": "infrastructure",
  "run_root": "/path/to/project/.qa/artifacts/scenarios/visual_occlusion_demo/20260502T120000Z-attempt-2",
  "attempts": [
    {
      "attempt": 1,
      "status": "failed",
      "failure_type": "display_startup_exhausted",
      "category": "infrastructure",
      "run_root": "/path/to/project/.qa/artifacts/scenarios/visual_occlusion_demo/20260502T120000Z-attempt-1",
      "cleanup_status": {"status": "completed", "owned_processes_terminated": 1},
      "display_lifecycle": {
        "mode": "windowed",
        "display_driver": "x11",
        "virtual_display": {"wrapper": "xvfb-run", "display": ":99"},
        "startup_status": "exhausted",
        "shutdown_status": "completed"
      }
    },
    {
      "attempt": 2,
      "status": "failed",
      "failure_type": "virtual_display_allocation_failed",
      "category": "infrastructure",
      "run_root": "/path/to/project/.qa/artifacts/scenarios/visual_occlusion_demo/20260502T120000Z-attempt-2",
      "cleanup_status": {"status": "completed", "owned_processes_terminated": 0},
      "display_lifecycle": {
        "mode": "windowed",
        "virtual_display": {"wrapper": "xvfb-run", "allocation_status": "failed"},
        "startup_status": "failed"
      }
    }
  ],
  "retry_guidance": {
    "eligible": true,
    "policy": "opt_in_bounded_startup_only",
    "attempted": true,
    "exhausted": true,
    "next_command": "godot-qa scenario bundle run screenshot_heavy_regression_gate --retry-startup 2 --retry-startup-backoff-ms 500 --json"
  }
}
```

Example bundle-run failure payload:

```json
{
  "ok": false,
  "command": "scenario bundle run",
  "message": "scenario bundle run failed",
  "data": {
    "bundle": "malformed_bundle_demo",
    "path": "/path/to/project/qa/gates/malformed_bundle_demo.yaml",
    "artifacts": {
      "root": "/path/to/project/.qa/artifacts/bundles/malformed_bundle_demo/20260502T120000Z",
      "meta": "/path/to/project/.qa/artifacts/bundles/malformed_bundle_demo/20260502T120000Z/meta.json",
      "summary": "/path/to/project/.qa/artifacts/bundles/malformed_bundle_demo/20260502T120000Z/summary.json",
      "result": "/path/to/project/.qa/artifacts/bundles/malformed_bundle_demo/20260502T120000Z/result.json"
    }
  },
  "failures": [
    {
      "type": "missing_scenario",
      "command": "scenario bundle run",
      "expected": "existing scenario file by declared name or stem",
      "actual": "missing_required_scenario_for_bundle_demo",
      "path": "/path/to/project/qa/gates/malformed_bundle_demo.yaml"
    }
  ]
}
```

Recursive or cyclic `required_bundles` fail with `bundle_cycle`, unsafe included bundle names fail before expansion, missing included bundles use a missing-bundle failure, and duplicate required scenarios report the first and duplicate `source_bundle` values. Missing optional scenarios remain warnings by default and fail only with `--strict`.
The new artifact index uses only normalized public names and does not add project-specific gate names. Legacy `child_run_root` or `launch_args` fields are not part of `artifact-index.json`.

## Live gate commands

```bash
godot-qa live-gate list --project <project> --json
godot-qa live-gate run <name> --project <project> [--strict] [--session <id>] [--headless|--windowed] --json
```

Live gates are CLI-owned validation profiles loaded from `qa/live_gates/`. They start or attach to a normal authenticated live session, keep debug overlays hidden unless the gate explicitly configures otherwise, inspect durable anchors, call allowlisted read-only QA methods/probes through the runtime bridge, read bridge errors, and write diagnostic artifacts under `.qa/artifacts/live-gates/<gate>/<run-id>/`. Generated artifacts are evidence for review and debugging; they do not approve or update screenshot baselines.

`live-gate list` supports `--project <project>` and `--json`.

`live-gate run` options:

- `<name>` resolves a manifest by declared `name` or file stem under `qa/live_gates/`.
- `--project <project>` selects the Godot project root.
- `--session <id>` attaches to an existing live session instead of starting one.
- `--strict` promotes optional-anchor warnings to failures.
- `--headless` and `--windowed` mirror live-session launch display selection when the CLI starts the session.
- `--json` emits the stable result envelope.

Sample-project commands:

```bash
cargo run --bin godot-qa -- live-gate list --project examples/sample_project --json
cargo run --bin godot-qa -- live-gate run hidden_overlay_inspection --project examples/sample_project --json
```

Example list success payload:

```json
{
  "ok": true,
  "command": "live-gate list",
  "message": "found 1 live gate(s)",
  "data": {
    "gates": [
      {
        "name": "hidden_overlay_inspection",
        "path": "/path/to/project/qa/live_gates/hidden_overlay_inspection.json",
        "description": "Verify hidden-overlay runtime anchors"
      }
    ]
  },
  "failures": []
}
```

Example run success payload:

```json
{
  "ok": true,
  "command": "live-gate run",
  "message": "live gate passed",
  "data": {
    "gate": "hidden_overlay_inspection",
    "session": {
      "id": "20260506T120000Z",
      "attached": false,
      "status": "running"
    },
    "overlay": {
      "expected": "hidden",
      "actual": "hidden",
      "evidence": "qa:debug_overlay.get_qa_overlay_state"
    },
    "anchors": [
      {
        "name": "main_menu",
        "locator": "qa:main_menu",
        "required": true,
        "status": "passed",
        "resolved": "path:/root/MainMenu",
        "methods": ["get_qa_menu_state"],
        "probes": []
      }
    ],
    "artifacts": {
      "root": "/path/to/project/.qa/artifacts/live-gates/hidden_overlay_inspection/20260506T120000Z",
      "summary": "/path/to/project/.qa/artifacts/live-gates/hidden_overlay_inspection/20260506T120000Z/summary.json",
      "errors": "/path/to/project/.qa/artifacts/live-gates/hidden_overlay_inspection/20260506T120000Z/errors.json"
    },
    "warnings": []
  },
  "failures": []
}
```

Failure behavior:

- `live-gate list` exits non-zero when `qa/live_gates/` is missing, manifests are malformed, gate names are duplicated, or paths are unsafe.
- `live-gate run` exits non-zero when the gate is missing or ambiguous, required anchors cannot resolve, an optional anchor is missing under `--strict`, a QA method is rejected, a probe fails, `session.status` reports stale or invalid session metadata, `errors.read` reports runtime errors, or overlay-hidden verification cannot be proven through documented project-owned QA surfaces.
- Optional anchors produce structured warnings by default and do not hide required-anchor failures.
- Failure payloads identify `command: "live-gate run"`, the gate name, anchor name, locator, expected state, actual state, bridge command when applicable, and artifact paths when available.

## Android live sessions

`session start` accepts an additive target contract:

```text
godot-qa session start \
  --target android \
  --preset <android-debug-preset> \
  [--device <adb-serial>] \
  [--package <application-id>] \
  [--scene res://...] \
  [--project-arg <value>]... \
  [--no-apk-cache] \
  --json
```

`--target` defaults to `desktop`, preserving all existing desktop syntax and
defaults. Android requires `--preset`. `--device` is optional only when ADB
reports exactly one authorized device. `--package` overrides the package derived
from the preset for custom Gradle configurations. ADB resolution order is `ADB`,
`ANDROID_SDK_ROOT`, `ANDROID_HOME`, then `PATH`.

Android rejects `--headless`, an explicit `--viewport`, `--hot-reload`,
`--fastest`, `--fixed-fps`, and `--no-audio`. These conflicts return a non-zero
structured failure with `kind: "android_option_unsupported"`. Preset and device
validation additionally use stable failure kinds including
`android_preset_required`, `android_export_preset_invalid`, `android_package_invalid`,
`android_device_not_found`, `android_device_ambiguous`,
`android_device_unauthorized`, and `android_device_unavailable`.

Start performs debug APK export, `adb install -r`, private stdin bootstrap
provisioning, an exact per-session `adb forward`, a fresh launcher start, device
PID discovery, a host logcat sidecar, and an authenticated bridge probe. The APK
is deleted after successful installation and retained as `retained_apk` when
installation or immediate device revalidation fails. A wireless `host:port` or
discovered ADB mDNS service serial is reconnected after export because an
exporter may restart the ADB daemon; pairing is not managed.

Start caches the debug-APK export so an unchanged project skips the export on the
next start. The cache key is a SHA-256 of the Godot binary path and version, the
preset name, the resolved package, and a sorted manifest of project file paths,
sizes, and mtimes; the manifest mirrors the exporter's exclusions (hidden `.`
directories such as `.git`/`.godot`, any directory holding a `.gdignore`, and the
artifacts directory when it resolves inside the project root). Entries live at
`.qa/cache/android/<key>/session-debug.apk` with a `meta.json` sidecar. A hit
copies the cached APK into the session deployment directory, so
install/`retained_apk`/cleanup behavior is unchanged; the newest four keys are
retained and older ones are evicted. The cache is self-healing: a read error
(corrupt or mismatched entry) degrades to a fresh export, and a write error is a
non-fatal deployment-log warning. `--no-apk-cache` bypasses the read docker-style
while the fresh export still refreshes the entry. Results add
`apk_cache: {status, key}` where `status` is `hit`, `miss`, or `bypassed`, and the
deployment log records an `apk-cache: hit|stored|bypassed` note.

Successful start and status results include `target: "android"`, device serial,
package, preset, device PID and port, forwarded host endpoint, actual viewport,
`deployment_mode: "debug_apk"`, artifact directory, deployment log, and logcat
path. Session metadata also records the resolved ADB executable and private
device artifact root, but never the session token.

Android `session stop` first requests `session.stop`, then falls back to
`am force-stop`, removes only the recorded forward, stops the logcat helper,
removes only the session's private QA directory, and clears local metadata after
successful cleanup. A disconnected device is reported as unavailable and its
recoverable metadata is retained. Plain `session reload` reloads packaged device
content. `session watch` and script/resource/structural host-file reload return
explicit Android unsupported failures.

`edit-live set/call/diff/export-patch` work over the forwarded bridge on Android
exactly as on desktop: the commands carry scalar params and return scalar
results, so the transport passes them through unchanged with no device artifact
pull. Edits are session-temporary. A packaged-content `session reload` restores
the scene and clears the in-memory edit ledger, so `export-patch` must run before
a reload to capture any edits. `export-patch` writes the patch host-side from the
response data (no device file round-trip), and `apply-patch` remains a
desktop/host operation.

### Android one-shot scenario execution

`scenario run <name> --target android` (also `--file` scenarios) deploys a
one-shot scenario-mode session, runs a single scenario on a real device, and
tears the session down. It reuses the `session start --target android` deploy
stages (debug-APK export through the cache, `adb install -r`, private bootstrap,
exact forward, launch, PID discovery, logcat sidecar, bridge probe) and drives
the scenario through the authenticated `scenario.run` bridge command.

```text
godot-qa scenario run <name> \
  --target android \
  --preset <android-debug-preset> \
  [--device <adb-serial>] \
  [--package <application-id>] \
  [--no-apk-cache] \
  [--audio] \
  --json
```

Honored options: `--preset`/`--device`/`--package`/`--no-apk-cache` (deploy
inputs, identical to `session start`), `--audio` (device audio is real),
scenario-declared `project_args`, and every runtime feature the device supports — save_screenshot,
visual pixel and temporal assertions, audio capture, `time_control`, and
wall-clock timeouts. Rejected with `android_option_unsupported`: `--headless`,
`--windowed`, `--no-audio`, `--visual-risk`, and `--visual-risk-strict` (the host
cannot control the device display, and visual-risk baselines are host-viewport).
A missing `--preset` returns `android_preset_required`; passing any of
`--preset`/`--device`/`--package`/`--no-apk-cache` with `--target desktop`
returns `invalid_params`.

A scenario preflight rejects scenarios that cannot run faithfully on device with
`android_scenario_unsupported` and a `reason`: `godot_args` (process-launch
flags), `fixed_fps` (an explicit `determinism.fixed_fps`), `viewport_override`
(an explicit `viewport`), `multi_viewport` (a `viewports` matrix),
`layout_matrix` (a layout-audit matrix), or `screenshot_match` (host-viewport
baselines are meaningless at device resolution). A live Android session blocks a
one-shot run with `android_session_active` ("run `session stop` first"); a
desktop live session does not conflict.

Timeouts are wall-clock only. The one-shot strips `run.frame_budget` from the
pushed request because device pacing is real, so the scenario's declared timeout
(or the default) is enforced purely as a transport wall clock with no device-side
frame budget.

The device writes result.json and evidence privately under
`user://godot_qa/live/<session-id>/run`; the CLI pulls them into the normal host
`.qa/artifacts` run tree and returns the same result envelope a desktop run
returns, plus `target: "android"` and an `android` object with `session_id`,
`device`, `package`, `preset`, `device_pid`, `forwarded_host_port`,
`device_artifact_root`, `deployment_log`, `logcat`, `deployment_mode: "debug_apk"`,
and `cleanup_complete`. The deploy honors the debug-APK cache exactly like
`session start` (surfaced as `apk_cache` on the deployment log). Cleanup always
runs; a failed cleanup step sets `cleanup_complete: false` and fails the run. A
failed `scenario.run` round-trip is classified by probing the device: a still-
running app is `scenario_run_timeout`, a vanished app is `android_app_crashed`
(naming the retained logcat and deployment log), and an unreachable device is
`android_device_unavailable`.

Example one-shot command:

```bash
godot-qa scenario run sample_main_menu_start --target android --preset "Android QA" --json
```

The first Android live scope includes inspection, input, assertions,
screenshots, snapshots, audio, profiling, traces, pause/step, live edit,
packaged-scene reload, and one-shot scenario execution. It excludes release/AAB deployment,
Android `check`/`scenario bundle`/`scenario sweep`/`scenario profile` execution,
hot reload/watch, warm pools, headless mode, fastest or fixed pacing, and
host-controlled viewport resizing.

## Verified asset trials

Asset trials test selected source media in an existing saved scene, leave the
temporary binding change available for review, and then keep or discard it. A
trial manifest uses `godot-qa.asset-trial.v1`, names one edited scene, exactly
one validation scenario or scenario file, and one or more direct native
resource bindings. Version 1 supports PNG textures on `Sprite2D.texture` or
`TextureRect.texture`, and PCM16 mono/stereo WAV streams on
`AudioStreamPlayer.stream` or `AudioStreamPlayer2D.stream`.

```text
godot-qa asset try <manifest> --project <project> --json
godot-qa asset status [<trial-id>] --project <project> --json
godot-qa asset keep <trial-id> --project <project> --json
godot-qa asset discard <trial-id> --project <project> --json
```

The JSON/YAML manifest requires `format: godot-qa.asset-trial.v1`, `scene`,
`validation` with exactly one of `scenario` or `scenario_file`, and a nonempty
`bindings` list. Each binding declares a unique `id`, candidate `source`, new
`destination`, scene-relative `edit_path`, runtime `runtime_target`, `property`,
and `evidence`. Evidence names exactly one `snapshot` or `audio` capture and
its adjacent `state_before` / `state_after` checkpoints. Relative paths resolve
against the selected project, including the manifest path. A runtime target
identifies the verified instance even when other instances share the edited
scene. Inherited edited scenes and targets inside nested instances are rejected.

PNG sources are limited to 4096×4096 pixels and require lossless imports. WAV
sources must be PCM16 mono/stereo with positive duration at most 30 seconds.
Both formats have a 32 MiB source limit. A new `try` after discard uses fresh
content hashes even if the candidate source filename stays the same.

`try` validates the manifest and source media, captures baseline evidence,
imports media into absent project-local destinations, applies temporary scene
bindings, and runs one ordinary validation scenario in a fresh Godot process.
It leaves the applied scene available for inspection. `status` reports a named
trial, or lists journals and the active reservation when no ID is given.
`keep` accepts a passing trial;
`discard` restores the original scene and removes only files owned by that
trial. Discard also recovers interrupted trials. A failed validation cannot be
kept, and neither action changes screenshot baselines.

Trial state and original scene backups live under the selected project's
`.qa/asset-trials/<trial-id>/` and must be retained while the trial is active.
Trial evidence lives under the configured artifact root in
`asset-trials/<trial-id>/before` and `after`; these directories are retained for
audit independently of ordinary artifact pruning. Media destinations must be
new paths with existing parent directories. Discard retains changed or possibly
referenced media with an explicit warning; those paths cannot be reused until
the user resolves them or chooses new destinations. The conservative source scan
recognizes whole loader identifiers: unrelated helper calls such as
`_relay_payload(...)` do not prevent cleanup. Dynamic loader calls, direct resource
references, and native extensions still retain possibly referenced media; comments
and string contents are scanned conservatively. The sample manifest and validation scenario are
`examples/sample_project/qa/asset-trials/coin.yaml` and
`examples/sample_project/qa/scenarios/examples/asset_coin_pickup.yaml`.

JSON reports `state`, `phase`, `technical_status`, `review_status`, retained
artifact paths, and recovery guidance separately. Pixel differences are review
evidence; gameplay and binding failures block keep. Keep rechecks dependencies,
source and imported media, execution configuration, and retained evidence.
During idle review, ordinary runtime previews are allowed. Import, source
writes, hot reload, watch, and addon installation are blocked; keep and discard
also refuse running previews. Close external editors before starting a trial.
