# Runtime Bridge Protocol

The bridge protocol is the JSON contract between the CLI and the running Godot project.

## Principles

- JSON-compatible values only.
- One command in, one response out.
- Every response includes `ok`.
- Responses echo `id` when the request supplied one.
- Every failure includes `error.type` and `error.message`.
- Live commands require a session token.
- The protocol should be transport-agnostic: JSON lines over TCP for live mode, JSON files for one-shot mode.

## Request envelope

```json
{
  "protocol_version": "0.1",
  "token": "opaque-session-token",
  "id": "cmd-0001",
  "command": "snapshot.capture",
  "params": {
    "name": "after_start"
  }
}
```

## Success response

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "id": "cmd-0001",
  "data": {
    "snapshot": "after_start",
    "artifacts": {
      "screenshot": ".qa/artifacts/live/latest/snapshots/after_start/screenshot.png",
      "tree": ".qa/artifacts/live/latest/snapshots/after_start/tree.json",
      "ui": ".qa/artifacts/live/latest/snapshots/after_start/ui.json"
    }
  }
}
```

## Error response

```json
{
  "ok": false,
  "protocol_version": "0.1",
  "id": "cmd-0001",
  "error": {
    "type": "target_not_found",
    "message": "No node/control matched qa:start_button",
    "data": {
      "target": "qa:start_button",
      "candidate_status": "none"
    }
  }
}
```

For `qa:` locators, `target_not_found` preserves the stable error type. `error.data` may add `candidate_status` and `rejected_candidates` diagnostics when fast-path group candidates were stale, freed, outside the scene tree, or mismatched before fallback scanning failed.

## Command queueing and concurrency

The bridge executes live commands strictly one at a time. Overlapping commands (for example a `trace.record` arriving while a `surface.audit_probe` is still in flight) are not rejected on arrival: each incoming command takes a monotonically increasing FIFO ticket and awaits its turn, then runs to completion before the next queued command begins. Ordering is fair by arrival, and per-command execution is still bounded by whatever deadline the command itself already enforces.

The queue is bounded so a stuck or long-running command cannot back up callers indefinitely. A command still returns the compatible `type: "session_bridge_busy"` error when:

- the pending queue is already at its depth bound (`reason: "queue_full"`), or
- a command waits longer than the queue-wait bound (default 30s) for its turn (`reason: "queue_wait_timeout"`).

`session_bridge_busy` responses keep their existing shape and remain retryable by the CLI with backoff. `error.data` additionally reports `reason`, `queue_depth`, `waited_msec`, and the `in_progress` diagnostics (including `elapsed_msec`, and `frames`/`seconds` where applicable) so the CLI can compute an informed backoff. Authentication and configuration failures (`bridge_inactive`, `session_token_required`, `invalid_session_token`, `session_token_unconfigured`) are still evaluated before queueing and are never reported as bridge busy.

An idle live session throttles its own serve loop: with no command in flight it sleeps about 1 ms per frame and polls the file transport only every 8th idle frame, so an idle session accepts a TCP command at most ~1 ms later and picks up a file-transport request at most ~8 ms later than before, while a session with a command in flight is never delayed and keeps polling every frame.

## Bootstrap behavior in S2

The currently implemented Godot-side helpers only cover the bootstrap envelope and inactive-bridge behavior. When the addon is installed but QA mode is not enabled, commands should fail like this:

```json
{
  "ok": false,
  "protocol_version": "0.1",
  "id": "cmd-0001",
  "error": {
    "type": "bridge_inactive",
    "message": "QA mode is not enabled",
    "data": {
      "command": {
        "id": "cmd-0001",
        "command": "session.status"
      }
    }
  }
}
```

The live TCP/session transport is implemented in S8 on the Godot side. The current migration path keeps that runtime contract unchanged while moving the CLI-side live client into Rust (`godot-qa-runtime`) in S30. The bridge-local command implementations below are shared by one-shot runs and the live session server.

## Surface Audit Probe

### `surface.audit_probe`

`surface.audit_probe` is the runtime bridge command behind CLI `surface audit`. It is provider-driven and read-only: the bridge resolves the requested provider target, calls only an approved surface provider method, and returns bounded sample evidence. It must not mutate scene state, navigation data, collision data, or project geometry, and it must not draw or enable a debug overlay.

Request params:

```json
{
  "provider": "qa:surface_provider",
  "provider_method": "qa_surface_probe",
  "mode": "region",
  "center": "qa:clear_spawn",
  "region_size": {"width": 320.0, "height": 240.0},
  "sample_step": 16.0,
  "actor_radius": 24.0,
  "min_spawn_clearance": 72.0,
  "strict_warnings": false
}
```

Approved provider method names are `qa_surface_probe`, `get_qa_surface_state`, `get_surface_qa_state`, and `qa_probe_surface`. The generic runtime must not call arbitrary methods. Providers may support one-point or batch input, but the bridge contract remains bounded by CLI/runtime point limits; excessive batches fail with `surface_audit.sample_limit_exceeded` evidence rather than scanning unbounded world space.

Provider response for one point:

```json
{
  "point": {"x": 10.0, "y": 20.0},
  "surface": "walkable",
  "solid": false,
  "soft": false,
  "cost": 1.0,
  "drag": 0.0,
  "push_strength": 0.0,
  "metadata": {}
}
```

Success response data includes `command: "surface.audit_probe"`, the provider and method, bounded `samples`, anchors or route metadata when applicable, and any provider-side warnings. Project semantics stay in the provider: labels such as `water`, `floor`, `road`, or `danger` are reported as project-defined facts unless the provider also marks `solid`, `soft`, or related numeric fields. The bridge does not infer gameplay meaning from labels, node names, colors, textures, or collision layers.

## Live session reload

### `session.reload_scene`

Request:

```json
{
  "command": "session.reload_scene",
  "params": {}
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "scene": "res://ui/main_menu.tscn",
    "old_root": "/root/MainMenu",
    "new_root": "/root/MainMenu2",
    "paused": false
  }
}
```

The runtime reloads the live session scene path with deep cache replacement, swaps in the new scene root, clears held input/live-edit/runtime-error state, preserves paused state, and leaves the QA bridge/session process running. If loading or instantiation fails, the old scene remains active and the command returns `scene_load_failed` or `scene_instantiate_failed`.

## Scenario profiling

`scenario profile` is a one-shot scenario runner extension, not a new live TCP bridge command. The CLI prepares the same one-shot scenario request used by `scenario run` and adds a profiling request flag for the Godot runtime runner. The runtime returns profiling data in the one-shot result payload, and the CLI summarizes it plus writes the bounded `profile.json` artifact.

`check --tests --profile` uses the same one-shot profiling model for manifest-backed tests. It is a `qa_runner.gd` request/payload extension for one-shot check execution, not a new live bridge command and not an alias for `session.profile`. The CLI selects manifest entries, launches the normal one-shot runner, asks the runtime to include profile data for each selected test, validates the returned payload, summarizes it into command JSON, and writes the aggregate check `profile.json` artifact.

Profiling data comes from Godot/runtime counters and runner-owned timing:

- frame-time samples collected by the runtime runner
- Godot `Performance` monitor counters when available
- per-step timings for setup, input, waits, snapshots, assertions, runtime requests, manifest test startup/script execution/teardown, artifact writes, and runner phases
- opt-in `top_script_methods` spans recorded by project code through `res://addons/godot_qa/qa_profiler.gd`

The protocol does not define a host profiler transport, does not shell out to Linux `perf`, and does not require elevated host profiler permissions. Optional monitor counters that Godot cannot provide should be reported as unsupported or omitted without failing an otherwise valid profile. Unwrapped script code returns an available-but-empty `top_script_methods.methods` list rather than failing. Monitor unavailability is diagnostic evidence; malformed required frame/timing payloads, runtime failures, assertion failures, script errors, process failures, timeouts, and artifact write failures still fail the owning `scenario profile` or `check --tests --profile` command.

The Godot Editor Profiler export is intentionally out of this runtime protocol. `godot-qa profile editor export --json` reads the editor plugin artifact with `source: "editor_script_functions"` from `.qa/artifacts/editor-profiler/latest.json`; it does not use the localhost runtime bridge, session token, or game runtime command channel. That source remains explicitly experimental and version-aware because it captures raw editor debugger profiler messages instead of a stable runtime API.

## Host process ownership metadata

The host persists optional `process_identity` in Windows session records and `.qa/debug_host.json`: an opaque unsigned 64-bit Windows FILETIME creation timestamp paired with the PID. Unix records omit the field; older records remain readable. This is host-side lifecycle metadata, not a new runtime command, token, or bridge request field. A mismatched creation identity is treated as stopped/stale, and fallback termination never targets a reused PID. Without identity, legacy Windows records can use an authenticated bridge stop but cannot authorize fallback process termination.

## One-shot Frame Pacing And Time Control

Default fastest frame pacing is process launch metadata owned by the CLI, not a live TCP bridge command and not Godot-owned runtime state. For automated `scenario run` and manifest-backed `check --tests`, the CLI may launch Godot with generated `--fixed-fps <n>` and `--disable-vsync` flags and report `frame_pacing` in command/result artifacts. Live sessions remain realtime by default, but `session start --fastest` and live-script `session_start.frame_pacing` may opt into the same generated launch flags. If `time_control` is present, generated frame pacing is disabled for that one-shot run.

Live fastest pacing metadata is persisted in `.qa/session.json` and surfaced by `session status`, `session list`, `session start`, and live-script step results as additive `frame_pacing` evidence. The metadata includes `mode: "fastest"`, `fixed_fps`, `source`, `project_setting`, and `generated_godot_args`. Reusing a live session does not change its process launch flags; live scripts that request a different pacing than the active session fail with `session_frame_pacing_mismatch`.

`time_control` is a one-shot scenario/check request field, not a live-session default. The CLI may include it in the scenario or check payload sent to `qa_runner.gd` after validating the schema:

```json
{
  "scenario": {
    "name": "accelerated_timer_demo",
    "time_control": {
      "time_scale": 8.0,
      "physics_ticks_per_second": 480,
      "max_physics_steps_per_frame": 16
    }
  }
}
```

The Godot addon owns effective runtime state. It captures original Engine settings, applies requested `Engine.time_scale`, optional `Engine.physics_ticks_per_second`, and optional `Engine.max_physics_steps_per_frame`, then restores them before final one-shot result reporting whenever the runtime remains alive. Successful runtime results may include:

```json
{
  "time_control": {
    "original": {"time_scale": 1.0, "physics_ticks_per_second": 60, "max_physics_steps_per_frame": 8},
    "requested": {"time_scale": 8.0, "physics_ticks_per_second": 480, "max_physics_steps_per_frame": 16},
    "effective": {"time_scale": 8.0, "physics_ticks_per_second": 480, "max_physics_steps_per_frame": 16},
    "restored": {"time_scale": 1.0, "physics_ticks_per_second": 60, "max_physics_steps_per_frame": 8},
    "restored_matches_original": true,
    "restoration_status": "restored",
    "warnings": []
  }
}
```

`warnings` can include `time_control_physics_pairing_missing` when accelerated visual/audio/physics-sensitive flows omit paired physics settings. Live TCP sessions do not inherit a prior one-shot `time_control`; a later live command should observe normal Engine settings unless the project itself changed them or an explicit live `session.set_time_scale` command changed the running session. Fastest live frame pacing is separate from `session.set_time_scale`: it controls launch-time simulated frame delta and vsync behavior, while `session.set_time_scale` changes persistent Godot Engine state in the active process.

## Live Session Time Scale

### `session.set_time_scale`

`session.set_time_scale` is a token-protected live TCP/file-transport bridge command for changing Godot Engine time settings in the already-running session. It is not inherited from one-shot `time_control` and does not auto-restore when the command returns.

Request params:

```json
{
  "time_scale": 4.0,
  "physics_ticks_per_second": 240,
  "max_physics_steps_per_frame": 16
}
```

`time_scale` is required and must be a positive finite number `> 0.0` and `<= 64`; `1.0` is the realtime scale. Optional `physics_ticks_per_second` and `max_physics_steps_per_frame` values must be integers `>= 1`. Invalid values return `type: "invalid_params"` and must identify the command, expected value, and actual value.

Successful responses include Godot-owned evidence:

```json
{
  "original": {"time_scale": 1.0, "physics_ticks_per_second": 60, "max_physics_steps_per_frame": 8},
  "requested": {"time_scale": 4.0, "physics_ticks_per_second": 240, "max_physics_steps_per_frame": 16},
  "effective": {"time_scale": 4.0, "physics_ticks_per_second": 240, "max_physics_steps_per_frame": 16},
  "persistent": true,
  "warnings": []
}
```

The setting persists in the live Godot process until another `session.set_time_scale` request, project code, scene reload behavior, or session shutdown changes it. Accelerated live sessions without both paired physics settings should return `time_control_physics_pairing_missing` warning evidence. Godot `Engine.time_scale` still does not accelerate audio playback speed.

## One-Shot Progress Evidence

Progress evidence for `scenario run` and manifest-backed `check --tests` is produced by the Godot-side runner (`qa_runner.gd`) during one-shot execution and written to `progress.jsonl`. It is artifact evidence, not a live-session command, and it does not change live TCP session token rules.

Before loading the scene, the addon writes a `runtime_started` progress event carrying the current run ID and nonce. Any progress from this attempt prevents startup retries; a later display-like error cannot turn gameplay or teardown into a retryable startup failure.

Scenario progress events are written before and after each runner-owned step or assertion. Each event identifies the active `step_index` or `assertion_index`, operation kind, safe target or locator context when present, `started_at`, `elapsed_ms`, and the run artifact paths needed to inspect partial evidence. Batch `check --tests` progress events identify the active manifest test entry with `id`, `kind`, `target`, state, `started_at`, `elapsed_ms`, and per-test or shared batch artifact paths.

The runtime must redact or omit sensitive launch context in progress events. Session tokens are never written to progress artifacts, and secret-looking project arguments follow the same persisted redaction contract as `meta.json` and `request.json` run-level metadata.

Timeout diagnostics use the most recent progress event to identify the active scenario step/assertion or manifest test. This evidence supports `check_test_timeout` and `scenario_run_timeout` failures and is also the source for informational `slow-summary.json` entries. Slow-summary thresholds do not affect bridge authentication, do not introduce retries, and do not make passing runs fail.

## Live session profiling

`session.profile` is a live TCP/file-transport bridge command for active sessions. It is token-protected like other live commands and samples the already-running Godot scene for a short bounded window.

Soak profiling adds short token-protected `session.profile.soak_start`, `session.profile.soak_status`, and `session.profile.soak_stop` requests. Start accepts the normalized version-1 `spec` and a safe `run_id`; the bridge chooses the artifact path beneath the active session root and allows one active soak. Status/stop require that ID and return the current bounded summary. The sampler runs from the bridge's ordinary process callback, so the bridge queue is not held for the soak duration. It writes each window before advancing and does not force a draw. Session status additionally reports runtime time scale, VSync mode, renderer and window size.

Request params:

```json
{
  "frames": 60,
  "max_frame_samples": 120,
  "top": 10
}
```

`seconds` may be sent instead of `frames`. The CLI validates bounds before sending the request; frame-based requests accept `1..=1000`.

Success data includes bounded `frame_times_ms`, `frame_samples`, Godot `Performance` `monitor_samples`, `unsupported_monitors`, `node_process_counts`, `resource_activity`, `top_script_methods`, `sample_window`, and `sample_limits`. `top_script_methods` is populated by opt-in spans recorded through `res://addons/godot_qa/qa_profiler.gd`; uninstrumented projects return an available-but-empty method list instead of failing. `resource_activity` should use `status: "unavailable"` unless reload/import/watch activity is observable from live-session state.

The runtime command name remains `session.profile`; S54 retry diagnostics do not introduce a replacement command or a token bypass. Requests and responses keep the existing token-protected live bridge envelope, and the CLI is responsible for the bounded retry loop before sampling begins.

Overlapping `session.profile` requests wait their turn in the bridge command queue (see "Command queueing and concurrency"). When the bridge still cannot accept `session.profile` because the queue is saturated or the queue-wait bound elapsed, it returns a structured busy/resource diagnostic that the CLI maps to `type: "session_bridge_busy"`. The diagnostic should identify `command: "session.profile"`, the contended resource when known, whether retry is eligible before sampling starts, and a short retry guidance string. Invalid tokens, unreachable sockets, connection timeouts, malformed profile payloads after sampling, and artifact write failures must not be reported as bridge busy.

## Live gate usage

`godot-qa live-gate run` is a CLI orchestration profile, not a new arbitrary scripting transport. A live gate starts or attaches to the existing token-protected live session and composes documented bridge commands:

- `session.status` verifies that the session metadata is current and the bridge is responsive.
- `node.inspect` resolves each configured durable anchor and records the runtime node path plus compatible inspection data.
- `assert.state_matches` calls allowlisted read-only QA state methods such as `get_qa_*_state()` or `get_*_qa_state()` on configured anchors.
- `assert.surface_probe` calls configured QA probe providers for bounded surface/point checks.
- `errors.read` collects runtime errors before the gate reports success.

Overlay-hidden verification stays project-owned. The CLI may request or verify hidden state only through documented QA anchors, state methods, probes, or metadata exposed by the project; it must not infer overlay state from raw `.tscn` files, hidden debug labels, screenshots alone, or arbitrary script execution. Projects without a supported overlay hook should return a clear warning or failure, never synthetic hidden-overlay evidence.

The live gate protocol boundary does not add `eval`, shell commands, filesystem write commands, or project-specific setup scripting to the bridge. Method and probe entries are allowlisted read-only observation surfaces and must return JSON-compatible values. Artifact writes remain controlled by the CLI artifact directory.

## Event trace bridge command

`event.trace` samples a bounded project-owned event provider during an active, token-protected live session. It is the bridge command behind `godot-qa event trace`; `trace record --events-provider` is not implemented in the current public command surface, so `event trace` is the supported event stream path for VFX/SFX/UI/collision correlation.

Request params:

```json
{
  "target": "qa:event_source",
  "provider": "get_qa_event_stream_state",
  "frames": 120,
  "sample_every_frames": 1,
  "max_events_per_sample": 256
}
```

Approved provider method names are `get_qa_event_stream_state`, `get_event_stream_qa_state`, `get_qa_events`, and `qa_event_stream`. Providers must be read-only, zero-argument unless a bounded argument contract is explicitly documented, and JSON-compatible. The runtime and CLI must not infer gameplay, audio, VFX, UI, or collision events from node names, logs, hidden debug labels, screenshots, engine audio internals, or engine VFX internals when no provider exists. The command must not add visible debug labels, event overlays, or gameplay mutation.

Success data includes bounded `events`, `summary`, `warnings`, `sample_window`, and `sample_limits`. Event type strings are project-provided and not interpreted by the tool. The required summary fields are `total_event_count`, `event_count_by_category`, `event_count_by_type`, `event_rate_per_second_by_type`, `max_events_in_one_frame`, `duplicate_event_count`, `missing_event_id_count`, `event_spam_warnings`, `paired_event_warnings`, `offscreen_event_warnings`, and `provider_error_count`.

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "events": [],
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
  }
}
```

Provider failures and malformed event payloads use stable warning IDs from the `event_trace.*` namespace: `event_trace.provider_missing`, `event_trace.provider_invalid`, `event_trace.event_missing_id`, `event_trace.event_not_json_compatible`, `event_trace.event_stream_truncated`, `event_trace.event_spam_risk`, `event_trace.paired_event_missing`, `event_trace.offscreen_event_risk`, `event_trace.provider_error`, and `event_trace.no_events_recorded`. Warnings become failures only when the CLI explicitly requests strict warning behavior.

## Telemetry bridge command

`telemetry.record` samples ONE read-only project-owned state source every N frames during an active, token-protected live session. It is the bridge command behind `godot-qa telemetry record`.

Request params:

- `provider` (string, optional): registered read-only state provider id, sampled through the provider registry.
- `target` + `method` (strings, optional): resolvable locator plus a zero-argument, read-only method on the resolved node (`qa_*` without the mutating `qa_prepare_/qa_apply_/qa_mutate_/qa_set_` prefixes, `get_qa_*_state`, or `get_*_qa_state`). Exactly one of `provider` or `target`+`method` is required.
- `frames` (int, clamped to `[1, 3600]`, default `600`) and `every` (int, clamped to `[1, 240]`, default `1`): sampling window and interval.

Success data contains `telemetry_id`, `session` frame/time bounds, the effective `config` (including `max_rows`), the collected `rows` (`{frame, t_ms, value}`; failed samples carry `value: null` plus an `error` status), `row_count`, `truncated`, `provider_error_count`, and bounded `telemetry.sample_failed` warnings. Row collection is capped at 3600 rows so the recording stays memory-bounded inside the game process. Validation failures use `invalid_params`, `telemetry.method_invalid`, and `telemetry.provider_missing`; provider-registry failures reuse the registry's structured errors.

## S6 shared runtime commands

These command names are the bridge-compatible contract shared by one-shot scenario execution and the live session transport. The shipped Rust CLI/runtime client preserves these names, and the Godot runtime is the source of truth for their behavior and payloads.

### `input.tap_action`

Request:

```json
{
  "command": "input.tap_action",
  "params": {
    "action": "ui_accept"
  }
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "action": "ui_accept"
  }
}
```

### `input.hold_action`

Request:

```json
{
  "command": "input.hold_action",
  "params": {
    "action": "ui_down",
    "frames": 4
  }
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "action": "ui_down",
    "frames": 4,
    "held": true
  }
}
```

### `input.release_action`

Request:

```json
{
  "command": "input.release_action",
  "params": {
    "action": "ui_down"
  }
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "action": "ui_down",
    "held": false
  }
}
```

### `input.click_target`

Request:

```json
{
  "command": "input.click_target",
  "params": {
    "target": "qa:start_button"
  }
}
```

`input.click_target` resolves its `target` through the shared runtime locator resolver. `qa:` targets use canonical `qa.id.<id>` groups first, compatibility `qa.<id>` groups second, then fallback scans for `qa_id` metadata and `qa_aliases`.

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "target": "qa:start_button",
    "resolved": "path:/root/MainMenu/MainMenuButtons/StartButton"
  }
}
```

### `input.mouse_wheel`

Request:

```json
{
  "command": "input.mouse_wheel",
  "params": {
    "x": 500,
    "y": 300,
    "dy": 3,
    "dx": 0
  }
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "device": "mouse",
    "position": {"x": 500, "y": 300},
    "dx": 0.0,
    "dy": 3.0,
    "buttons": ["wheel_down", "wheel_down", "wheel_down"]
  }
}
```

### `input.touch_down`

Request:

```json
{
  "command": "input.touch_down",
  "params": {
    "x": 640,
    "y": 360,
    "index": 0
  }
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "device": "touch",
    "index": 0,
    "position": {"x": 640, "y": 360},
    "pressed": true
  }
}
```

`input.touch_move`, `input.touch_up`, and `input.touch_tap` share the same request shape (`input.touch_up` may omit `x`/`y`, and the CLI then sends them as explicit `null`; `input.touch_move` also accepts `frames`). Their success `data` carries `device: "touch"`, `index`, and `position`, plus `relative` and `frames` for `touch_move`, `pressed: false` for `touch_up`, and optional `target`/`resolved` for `touch_tap`. See the touch prose in the command index for validation, the `MAX_INPUT_GESTURE_FRAMES` cap, the `touch_index_held`/`touch_index_not_held` error kinds, and held-state cleanup via `input.reset`.

### `input.drag`

Request (touch drag):

```json
{
  "command": "input.drag",
  "params": {
    "from_x": 600,
    "from_y": 300,
    "to_x": 200,
    "to_y": 300,
    "frames": 8,
    "pointer": "touch",
    "index": 0
  }
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "device": "touch",
    "pointer": "touch",
    "index": 0,
    "from": {"x": 600, "y": 300},
    "to": {"x": 200, "y": 300},
    "frames": 8,
    "steps": 8
  }
}
```

### `tree.dump`

Request:

```json
{
  "command": "tree.dump",
  "params": {
    "target": "qa:start_button",
    "max_depth": 1,
    "qa_only": true,
    "include_metadata": true,
    "metadata_keys": ["qa_id"]
  }
}
```

Success `data` matches the `tree.json` snapshot payload exactly.
All params are optional. `target` scopes capture to a resolved subtree, `max_depth` must be a non-negative integer, `qa_only` retains QA-bearing nodes plus readable ancestry, `include_metadata` controls metadata emission, and `metadata_keys` allowlists safe metadata keys. Invalid params fail with structured filter errors such as `invalid_max_depth`, `invalid_include_metadata`, `invalid_qa_only`, or `invalid_metadata_keys`; target resolution failures keep resolver types such as `target_not_found` and `ambiguous_target`.

### `ui.dump`

Request:

```json
{
  "command": "ui.dump",
  "params": {}
}
```

Success `data` matches the `ui.json` snapshot payload exactly.
That payload now includes all `Control` nodes under the scene subtree, with effective visibility captured in each entry's `visible` field.

## Temporal assertion protocol commands (S33/S52)

The runtime bridge accepts additive temporal assertion commands:

- `assert.temporal.max_delta_position`
- `assert.temporal.follows`
- `assert.temporal.never`
- `assert.temporal.within_limits`
- `assert.observe_until`

All temporal requests must include exactly one bounded window (`window_frames` or `window_seconds`) and may include `sample_every_frames` (default `1`). Requests may include `temporal_sampling` for assertion-local deterministic sampling; saved scenarios may also pass the scenario-level `determinism.temporal_sampling` setting through the same params object. Supported sampling modes are `fixed_delta` with `fixed_delta_seconds` and `max_frame_duration` with `max_frame_duration_seconds`.

### `assert.temporal.max_delta_position`

Request:

```json
{
  "command": "assert.temporal.max_delta_position",
  "params": {
    "target": "qa:demo_actor",
    "window_frames": 12,
    "max": 32,
    "sample_every_frames": 1
  }
}
```

Failure shape (example):

```json
{
  "ok": false,
  "error": {
    "type": "assertion_failed",
    "message": "temporal max delta position exceeded",
    "data": {
      "command": "assert.temporal.max_delta_position",
      "target": "qa:demo_actor",
      "window": {"frames": 12},
      "expected": {"max_delta_position": 32},
      "actual": {"max_delta_position": 57.2},
      "sample": {"frame": 118, "time_seconds": 1.966},
      "artifacts": {"temporal_evidence": ".qa/artifacts/.../temporal/max_delta_position-01.json"}
    }
  }
}
```

### `assert.observe_until`

`assert.observe_until` succeeds when one configured condition becomes true within the bounded window. It samples player-visible or QA-owned runtime state without consuming the whole gameplay timer before checking short-lived intermediate states.

Request:

```json
{
  "command": "assert.observe_until",
  "params": {
    "target": "qa:main_menu",
    "source": "qa:main_menu",
    "method": "get_qa_state",
    "matches": {
      "status": "pending"
    },
    "window_frames": 8,
    "sample_every_frames": 1,
    "temporal_sampling": {
      "mode": "fixed_delta",
      "fixed_delta_seconds": 0.0166667
    }
  }
}
```

Equivalent condition families:

- Property: `{"target":"qa:status_label","property":"text","operator":"contains","expected":"Pending","window_frames":12}`
- Visible: `{"target":"qa:spinner","visible":true,"window_seconds":0.25}`
- QA method state path: `{"target":"qa:main_menu","source":"qa:main_menu","method":"get_qa_state","matches":{"status":"pending"},"window_frames":8}`

Success response:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "command": "assert.observe_until",
    "target": "qa:main_menu",
    "condition": {
      "type": "state_matches",
      "matches": [
        {"path": "status", "expected": "pending"}
      ]
    },
    "window": {"frames": 8},
    "sample_every_frames": 1,
    "first_matching_sample": {
      "sample": 2,
      "frame": 4,
      "time_msec": 66,
      "target": "qa:main_menu",
      "matched": true,
      "actual": {
        "matched": [
          {"path": "status", "expected": "pending", "actual": "pending"}
        ]
      }
    },
    "observed_samples": [
      {"sample": 1, "frame": 2, "matched": false},
      {"sample": 2, "frame": 4, "matched": true}
    ],
    "artifacts": {
      "temporal_evidence": ".qa/artifacts/.../temporal/observe_until_qa_main_menu.json",
      "temporal_dir": ".qa/artifacts/.../temporal"
    }
  }
}
```

Failure response:

```json
{
  "ok": false,
  "error": {
    "type": "observe_until_mismatch",
    "message": "condition did not become true within the bounded window",
    "data": {
      "command": "assert.observe_until",
      "target": "qa:main_menu",
      "condition": {
        "type": "state_matches",
        "matches": [
          {"path": "status", "expected": "pending"}
        ]
      },
      "window": {"frames": 1},
      "sample_every_frames": 1,
      "observed_samples": [
        {"sample": 1, "frame": 1, "matched": false}
      ],
      "first_late_sample": {
        "sample": 2,
        "frame": 2,
        "matched": true,
        "actual": {
          "matched": [
            {"path": "status", "expected": "pending", "actual": "pending"}
          ]
        }
      },
      "why_no_match": "matched_after_window",
      "artifacts": {
        "temporal_evidence": ".qa/artifacts/.../temporal/observe_until_qa_main_menu.json",
        "temporal_dir": ".qa/artifacts/.../temporal"
      }
    }
  }
}
```

`why_no_match` is `matched_after_window` when `first_late_sample` matched after the allowed window; otherwise it identifies the observed mismatch class, such as no sampled condition match. The temporal evidence artifact contains the condition, window, sample cadence, optional `temporal_sampling`, all observed samples retained by the runtime, and the `first_matching_sample` or `first_late_sample` boundary evidence when available.

## Proximity assertion protocol commands (S34)

The runtime bridge accepts additive proximity assertion commands:

- `assert.distance`
- `assert.temporal.distance_changed`

These commands share locator resolution and failure categories with scenario/live assertions.
In direct bridge responses, structured mismatch fields are under `error.data`; scenario and CLI JSON surface the same fields on each top-level failure entry.

### Coordinate-space resolution rules

Target position resolution is runtime-owned and explicit:

- `Node2D`: `global_position` in `world_2d`.
- `Control`: `get_global_rect().get_center()` in `viewport_2d`.
- `Node3D`: `global_position` in `world_3d`.

Mixed coordinate spaces do not silently coerce values. If the runtime cannot safely compare the pair, the assertion fails with `unsupported_coordinate_space`.

### `assert.distance`

Request:

```json
{
  "command": "assert.distance",
  "params": {
    "a": "qa:actor",
    "b": "qa:target",
    "max": 96.0
  }
}
```

Outside-radius request example:

```json
{
  "command": "assert.distance",
  "params": {
    "a": "qa:actor",
    "b": "qa:target",
    "min": 96.0
  }
}
```

Failure shape (example):

```json
{
  "ok": false,
  "error": {
    "type": "distance_too_far",
    "message": "distance exceeded max threshold",
    "data": {
      "mismatch_type": "distance_too_far",
      "a": "qa:actor",
      "b": "qa:target",
      "a_resolved": "Main/QAProximityActor",
      "b_resolved": "Main/QAProximityTarget",
      "a_position": {"x": 100.0, "y": 20.0},
      "b_position": {"x": 220.0, "y": 20.0},
      "coordinate_space": "world_2d",
      "distance": 120.0,
      "threshold": {"max": 96.0},
      "expected": {"max": 96.0},
      "actual": {"distance": 120.0},
      "severity": "error"
    }
  }
}
```

Unsupported space failure shape (example):

```json
{
  "ok": false,
  "error": {
    "type": "unsupported_coordinate_space",
    "message": "distance assertion requires matching coordinate spaces",
    "data": {
      "mismatch_type": "unsupported_coordinate_space",
      "a": "qa:actor",
      "b": "qa:target",
      "a_resolved": "Main/QAProximityActor",
      "b_resolved": "Main/InventoryPanel",
      "a_position": {"x": 100.0, "y": 20.0},
      "b_position": {"x": 220.0, "y": 20.0},
      "expected": {"coordinate_space": "world_2d"},
      "actual": {"coordinate_space": "viewport_2d"},
      "severity": "error"
    }
  }
}
```

## Surface assertion protocol commands (S36)

The runtime bridge accepts additive surface assertion commands:

- `assert.surface_probe`
- `assert.surface_matches`

These commands are opt-in and project-owned. Runtime implementations must use explicit QA provider methods and declared surface labels; they must not infer semantics from collision layer names, physics body names, or arbitrary node names.

Provider method rules:

- Requests must include `target`. `surface_probe` scenario payloads include either a legacy `provider` locator or a registered `provider_id`. Provider IDs are explicit registry IDs, not node locators; they cannot be used for click, visibility, rect, or focus commands.
- With legacy target providers, `provider_method` must be QA-prefixed (for example `qa_probe_surface`) and can be supplied in params or by target metadata `qa_surface_probe_method`.
- Registered probe provider methods take exactly one request dictionary and return a JSON-compatible dictionary. The surface request includes bounded `points`, `coordinate_space`, optional `target`, declared `surfaces`, and `provider_id`.
- `surface_matches` reads actor state through `actor_surface_method` or target metadata `qa_surface_state_method`; that method must also be QA-prefixed.
- Provider responses must be JSON-compatible values only.
- Provider methods are read-only by convention and should not mutate gameplay state.

Bounded request rules:

- Requests are bounded by runtime point-count and radius limits.
- Out-of-bounds requests fail with actionable error types.

### `assert.surface_probe`

Request:

```json
{
  "command": "assert.surface_probe",
  "params": {
    "target": "qa:demo_actor",
    "provider": "qa:demo_actor",
    "provider_method": "qa_probe_surface",
    "coordinate_space": "world_2d",
    "surfaces": ["walkable", "blocked"],
    "points": [
      {"x": 120, "y": 420, "expected": "walkable"},
      {"x": 184, "y": 420, "expected": "blocked"}
    ]
  }
}
```

Registered provider request:

```json
{
  "command": "assert.surface_probe",
  "params": {
    "target": "qa:demo_actor",
    "provider_id": "surface_main",
    "coordinate_space": "world_2d",
    "surfaces": ["walkable", "blocked"],
    "points": [
      {"x": 120, "y": 420, "expected": "walkable"}
    ]
  }
}
```

Failure types (non-exhaustive):

- `missing_probe_provider`
- `invalid_probe_result`
- `unknown_surface_label`
- `unsupported_coordinate_space`
- `probe_request_out_of_bounds`
- `surface_mismatch`

### `assert.surface_matches`

Request:

```json
{
  "command": "assert.surface_matches",
  "params": {
    "target": "qa:demo_actor",
    "surface": "walkable",
    "provider": "qa:demo_actor",
    "provider_method": "qa_probe_surface",
    "coordinate_space": "world_2d",
    "actor_surface_method": "qa_current_surface"
  }
}
```

Response/evidence expectations:

- Success and failure payloads should include provider metadata and per-point evidence.
- `assert.surface_probe` point evidence uses `expected` from each requested point when present and `actual` on mismatches.
- `assert.surface_matches` evidence includes `point`, `coordinate_space`, `expected` (sampled world surface), `actual` (actor-reported surface), and provider/provider-method metadata.
- `assert.surface_matches` accepts optional `window_frames` or `window_seconds`, `sample_every_frames`, and `dwell_frames` or `dwell_seconds`; temporal failures include sample history.

### `assert.temporal.distance_changed`

Request:

```json
{
  "command": "assert.temporal.distance_changed",
  "params": {
    "a": "qa:actor",
    "b": "qa:target",
    "window_frames": 12,
    "min": 32.0,
    "sample_every_frames": 1
  }
}
```

Failure shape (example):

```json
{
  "ok": false,
  "error": {
    "type": "distance_changed_mismatch",
    "message": "distance changed less than the configured minimum",
    "data": {
      "mismatch_type": "changed_less_than_min",
      "a": "qa:actor",
      "b": "qa:target",
      "a_resolved": "Main/QAProximityActor",
      "b_resolved": "Main/QAProximityTarget",
      "coordinate_space": "world_2d",
      "window": {"frames": 12, "sample_every_frames": 1},
      "threshold": {"min": 32.0},
      "expected": {"min_delta": 32.0, "window": {"frames": 12, "sample_every_frames": 1}},
      "actual": {"delta": 8.0, "sample": 12},
      "initial_distance": 64.0,
      "peak_distance_delta": 8.0,
      "final_distance": 72.0,
      "sample": {
        "sample": 12,
        "distance": 72.0,
        "distance_delta": 8.0,
        "a_position": {"x": 100.0, "y": 20.0},
        "b_position": {"x": 172.0, "y": 20.0}
      },
      "failing_sample": 12,
      "samples": [],
      "artifacts": {"temporal_evidence": ".qa/artifacts/.../temporal/distance_changed-01.json"},
      "severity": "error"
    }
  }
}
```

### `focus.get`

Request:

```json
{
  "command": "focus.get",
  "params": {}
}
```

Success `data` matches the `focus.json` snapshot payload exactly.

### `focus.graph`

Request:

```json
{
  "command": "focus.graph",
  "params": {}
}
```

Success `data` matches the `focus.json` payload shape exactly. This stays separate from `focus.get` at the command level so live callers can ask for the graph explicitly without changing the existing focus-owner CLI contract.

### `node.inspect`

Request:

```json
{
  "command": "node.inspect",
  "params": {
    "target": "qa:start_button"
  }
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "target": "qa:start_button",
    "resolved": "path:/root/MainMenu/Canvas/MenuButtons/StartButton",
    "node": {
      "locator": "qa:start_button",
      "path": "/root/MainMenu/Canvas/MenuButtons/StartButton",
      "name": "StartButton",
      "type": "Button",
      "qa_id": "start_button",
      "groups": [],
      "inside_tree": true,
      "child_count": 0,
      "visible": true,
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
        "focus_mode": "all",
        "has_focus": true
      }
    }
  }
}
```

`node.inspect` is intentionally bounded: one node, one-hop relationships, and the same compact Control-state fields already exposed elsewhere. It is not a generic property browser.

`node.inspect` uses the same shared resolver as click and assertion commands. A missing `qa:` target keeps `type: "target_not_found"` and may include `candidate_status` plus `rejected_candidates`; an ambiguous `qa:` target keeps `type: "ambiguous_target"` and reports deduplicated `path:/root/...` matches in `error.data.actual`.

### `physics.inspect`

Request:

```json
{
  "command": "physics.inspect",
  "params": {
    "target": "qa:physics_probe",
    "radius": 300,
    "frames": 2,
    "state_provider": "scene-root",
    "state_methods": ["get_qa_physics_state"],
    "include_project_state": true,
    "snapshot": "collider-debug"
  }
}
```

`physics.inspect` is a token-protected live diagnostic command. It resolves `target` through the shared locator resolver, samples the active 2D scene without adding visible debug UI, and returns bounded physics evidence: target transform/visibility/velocity, nearby `CollisionObject2D` shape-owner reports, `PhysicsDirectSpaceState2D.intersect_shape` radius and target-shape overlap results, diagnostic classifications, and optional read-only project QA state. Project-state calls are limited to safe zero-arg methods such as `get_qa_*_state`, `get_*_qa_state`, and diagnostic `qa_*` methods; mutating setup/probe methods such as `qa_prepare_*` and `qa_apply_*` must be rejected.

The CLI writes the successful payload to `.qa/artifacts/live/<session-id>/physics/<capture-id>/physics.json` and reports that path as `data.artifacts.physics`. When `snapshot` is supplied, the bridge also captures the normal live snapshot bundle and returns it under `data.artifacts.snapshot`.

### `physics.overlay.collect`

Request:

```json
{
  "command": "physics.overlay.collect",
  "params": {
    "target": "qa:overlay_actor",
    "viewport": false,
    "radius": 400,
    "frames": 2,
    "include_aabb": true,
    "include_contacts": false,
    "state_provider": "scene-root",
    "state_methods": ["get_qa_collision_visual_state"],
    "include_project_state": true,
    "max_shapes": 512
  }
}
```

`physics.overlay.collect` is the Godot-owned metadata side of `godot-qa physics overlay`. The CLI composes it with screenshot capture: first it captures a real runtime screenshot through the normal screenshot command, then it calls this token-protected runtime command to collect screen-space collision and optional visual-footprint metadata. The CLI renders the final overlay PNG outside Godot from the screenshot plus this metadata. The runtime command itself must not add visible debug nodes, HUDs, labels, panels, or overlays to the game viewport.

The command resolves either a target locator or, when `viewport` is true, the scene root for viewport-wide collection. It samples after the requested frame delay, walks bounded `CollisionObject2D` evidence, projects supported 2D shape points/AABBs into screen space, and returns JSON-compatible `shapes`, optional `contacts`, optional `project_state`, and `metadata` summary counts. Shape entries should include stable facts when available: `logical_id`, `qa_id`, `node_path`, `node_type`, `shape_type`, `layer`, `mask`, `world_aabb`, `screen_aabb`, `screen_points`, `is_area`, `is_body`, and `is_soft`.

Optional project visual-footprint state is collected only through the same read-only QA allowlist used by physics diagnostics. Approved providers are zero-argument methods such as `get_qa_visual_footprint_state()`, `get_qa_collision_visual_state()`, `get_qa_*_state()`, `get_*_qa_state()`, or safe diagnostic `qa_*` methods. The bridge must reject missing providers, methods requiring arguments, mutating setup/probe methods such as `qa_prepare_*` or `qa_apply_*`, setter-style methods, runtime errors, and non-JSON-compatible return values through structured protocol errors.

Returned warnings are advisory unless the CLI is running `--strict`. Runtime warning IDs include `physics_overlay.no_collision_shapes_found` and `physics_overlay.unbounded_shape_count_truncated`; the CLI may add review/render warnings such as `physics_overlay.target_offscreen`, `physics_overlay.collision_without_visual_footprint`, `physics_overlay.visual_footprint_without_collision`, `physics_overlay.collider_extends_far_beyond_visual_footprint`, and `physics_overlay.collider_center_offset_from_visual_footprint`. Visual-footprint comparison warnings require approved provider data. If project visual footprints are unavailable, the runtime and CLI must omit or mark those comparisons not evaluated rather than guessing from sprite textures or names.

The collector is bounded by `max_shapes` and must truncate deterministically when the limit is exceeded, reporting `physics_overlay.unbounded_shape_count_truncated` with total and returned shape counts. It must not mutate collision shapes, gameplay state, project files, or baseline images.

### `trace.record`

Request:

```json
{
  "command": "trace.record",
  "params": {
    "targets": ["qa:trace_probe"],
    "frames": 60,
    "sample_every_frames": 1,
    "include_physics": true,
    "state_provider": "scene-root",
    "state_methods": ["get_qa_physics_state"],
    "include_project_state": true,
    "snapshot_start": false,
    "snapshot_end": false,
    "stop_after_state": {
      "target": "qa:race",
      "source": "method",
      "method": "get_qa_race_state",
      "matches": [{"path": "done", "operator": "equals", "expected": true}],
      "sample_every_frames": 1
    }
  }
}
```

`trace.record` is a token-protected live timeline command. It records observed runtime state from the running scene and does not create replayable input or video. The runtime resolves each target through the shared locator resolver, samples bounded frame/time, position, rotation, velocity, visibility, optional compact physics diagnostics, and optional safe read-only project QA state. The same read-only method allowlist as `physics.inspect` applies, and mutating setup/probe methods such as `qa_prepare_*`, `qa_apply_*`, and setters must be rejected before runtime dispatch. When explicit `state_methods` are supplied, the runtime preflights the state provider and methods before sampling; provider lookup failures, missing methods, argument-taking methods, runtime errors, and non-JSON-compatible values return structured errors through the normal protocol response.

`stop_after_state` is optional and uses the same read-only `state_matches` predicate shape as `audio.capture` and assertions. When present, the runtime polls the predicate once per captured sample, stops recording early on match, and echoes `stop_after_state` (`requested`, `matched`, `sample_count`, `stopped_at_frame`, `last_error`), `stop_condition_matched`, and `stop_condition_timed_out` in the payload. When the predicate never matches before `frames` elapses, the runtime still returns the full trace and the CLI reports `trace_record.stop_condition_timeout`.

The CLI writes the successful payload to `.qa/artifacts/live/<session-id>/traces/<trace-id>/trace.json` and reports that path as `data.artifacts.trace`. When start/end snapshots are requested, the bridge captures normal live snapshot bundles and returns them under `data.artifacts.snapshot_start` and `data.artifacts.snapshot_end`.

`physics collision-report` live collection is CLI-owned trace summarization over Godot-owned trace/provider evidence. The CLI requests bounded `trace.record` samples with physics diagnostics, optionally reads approved provider state, and writes `.qa/artifacts/live/<session-id>/collision-reports/<report-id>/collision_report.json`; the bridge remains the source of truth for scene tree, transforms, velocity, contacts, and provider return values. The approved optional provider names are `get_qa_collision_state()`, `get_collision_qa_state()`, `get_qa_physics_state()`, `get_physics_qa_state()`, and `qa_collision_probe()`. Providers must be zero-argument, read-only, JSON-compatible, and bounded, and the runtime must preserve the same safety boundary used by `trace.record` and `physics.inspect`: no gameplay mutation, no filesystem writes beyond controlled CLI artifact output, no unbounded sampling, no debug overlay injection, and no inference of soft/hard/trigger semantics from node or resource names.

### `errors.read`

Request:

```json
{
  "command": "errors.read",
  "params": {}
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "count": 1,
    "errors": [
      {
        "message": "Runtime error probe fired",
        "source": "fixture",
        "scene": "res://ui/fixtures/runtime_error_probe.tscn",
        "qa_id": "runtime_error_probe"
      }
    ]
  }
}
```

`errors.read` exposes the same structured QA runtime-error buffer used by `errors.json` and `assert.no_errors`. It does not attempt to capture every Godot log line or engine warning.

### Common error shapes

Command handlers should return structured errors such as:

- `session_token_unconfigured`
- `session_token_required`
- `invalid_session_token`
- `target_not_found`
- `ambiguous_target`
- `target_not_clickable`
- `input_action_missing`
- `action_not_held`
- `session_already_paused`
- `session_not_paused`
- `session_bridge_busy`
- `session_unreachable`
- `session_token_invalid`
- `session_connect_timeout`
- `profile_result_invalid`
- `artifact_write_failed`
- `edit_property_unsupported`
- `edit_property_value_invalid`
- `edit_method_unsupported`
- `patch_empty`
- `unsupported_command`

Each error should keep the actionable details in `error.data`, including the rejected `command`, `target` when relevant, and matching `path:/root/...` candidates for ambiguity cases.

Locator failure contracts keep their stable error types. `ambiguous_target` reports deduplicated `path:/root/...` matches in `actual`, while `target_not_found` may include fast-path diagnostics such as `candidate_status` and `rejected_candidates` without changing the error type.

For live profiling, `session_bridge_busy` means transient bridge/resource contention before runtime sampling began and may include `retry_eligible: true`, `resource`, `busy_command`, and `guidance` fields. `session_unreachable`, `session_connect_timeout`, and `session_token_invalid` are session configuration or authentication failures and should be surfaced distinctly. `profile_result_invalid` means the runtime returned a malformed or incomplete profile payload after profiling, and `artifact_write_failed` means the CLI could not write the validated profile artifact; both must not be retried as bridge-busy transport failures.

## Implemented live-session commands

### `session.status`

Request:

```json
{
  "command": "session.status",
  "params": {}
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "session_id": "20260421T120000Z",
    "scene": "res://ui/main_menu.tscn",
    "viewport": [1920, 1080],
    "started_at": "2026-04-21T12:00:00Z",
    "stop_requested": false,
    "paused": false
  }
}
```

### `session.pause`

Request:

```json
{
  "command": "session.pause",
  "params": {}
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "paused": true
  }
}
```

Error cases:

- `session_already_paused`

### `session.resume`

Request:

```json
{
  "command": "session.resume",
  "params": {}
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "paused": false
  }
}
```

Error cases:

- `session_not_paused`

### `session.step`

Request:

```json
{
  "command": "session.step",
  "params": {
    "frames": 1
  }
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "paused": true,
    "frames": 1
  }
}
```

`session.step` is a paused-session control, not a synonym for `session.wait_frames`. It requires the scene tree to already be paused, advances exactly the requested frame count, then restores paused state before replying.

Error cases:

- `invalid_params` for non-positive or malformed `frames`
- `session_not_paused`

### `session.wait_frames`

Request:

```json
{
  "command": "session.wait_frames",
  "params": {
    "frames": 10
  }
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "frames": 10
  }
}
```

### `session.stop`

Request:

```json
{
  "command": "session.stop",
  "params": {}
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "stopping": true
  }
}
```

### `snapshot.capture`

Request:

```json
{
  "command": "snapshot.capture",
  "params": {
    "name": "after_start",
    "target": "qa:start_button",
    "max_depth": 1,
    "qa_only": true,
    "include_metadata": true,
    "metadata_keys": ["qa_id"]
  }
}
```

`layout audit-current --live` does not introduce a separate bridge command. The CLI composes live layout audits by calling `snapshot.capture`, then loading the returned `tree`/`ui`/`focus`/`errors` artifact paths through the Rust CLI's own snapshot-loading helpers (`godot-qa-scenario::snapshot_context`) before running the existing layout policy. The artifact data itself is produced addon-side by the GDScript snapshot helpers in `addons/godot_qa/qa_snapshot.gd`.
`snapshot.capture` accepts the same optional filter params as `tree.dump`; `name` is consumed only for artifact placement. Filters affect JSON artifacts only. The screenshot artifact is a full viewport capture and is never cropped or filtered. Invalid filter params fail before writing the snapshot bundle when possible, and unresolved targets return the normal target failure contract.

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "name": "after_start",
    "artifacts": {
      "root": "/path/to/.qa/artifacts/live/20260421T120000Z/snapshots/after_start",
      "screenshot": "/path/to/.qa/artifacts/live/20260421T120000Z/snapshots/after_start/screenshot.png",
      "tree": "/path/to/.qa/artifacts/live/20260421T120000Z/snapshots/after_start/tree.json",
      "ui": "/path/to/.qa/artifacts/live/20260421T120000Z/snapshots/after_start/ui.json",
      "focus": "/path/to/.qa/artifacts/live/20260421T120000Z/snapshots/after_start/focus.json",
      "errors": "/path/to/.qa/artifacts/live/20260421T120000Z/snapshots/after_start/errors.json"
    }
  }
}
```

### `scenario.run`

Runs a whole scenario in-process on a live (warm) session. The CLI has already written the scenario's `request.json`; the addon loads the scene, runs steps and asserts, and writes the same `result.json` a cold run writes. The response is liveness only — the CLI reads the verdict from `result.json` on disk.

Request:

```json
{
  "command": "scenario.run",
  "params": {
    "request_path": "/path/to/.qa/artifacts/main_menu/20260421T120000Z/request.json",
    "run_id": "20260421T120000Z",
    "timeout_seconds": 60.0,
    "timeout_frames": 900
  }
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "scenario": "main_menu",
    "ok": true,
    "write_ok": true,
    "result_path": "/path/to/.qa/artifacts/main_menu/20260421T120000Z/result.json",
    "failure_count": 0
  }
}
```

`request_path` is required. `timeout_seconds` and `timeout_frames` are the transport/frame budget for the round-trip.

`run_id` is optional and carries the CLI run id for this scenario (the same id as `run.id` in `request.json`). When it is a non-empty string the addon brackets the entire handler with two marker lines written to **stderr**:

```text
[godot-qa] scenario-log begin <run-id>
[godot-qa] scenario-log end <run-id>
```

The `begin` marker is printed before the scenario executor is constructed and the `end` marker after it returns, on every exit path including early request errors — so a well-formed window always exists. Markers are printed with `printerr` rather than `print` deliberately: stderr is the same unbuffered stream engine `ERROR:`/`WARNING:` lines use, so a line between the two markers provably belongs to that scenario. A pooled warm session appends all its scenarios to one `logs.txt`, and these markers are what let the CLI cut out the slice belonging to a single run. When `run_id` is absent or empty (a cold run, or an older CLI) no marker is printed at all, so those logs stay byte-identical.

### `logs.read`

Request:

```json
{
  "command": "logs.read",
  "params": {
    "since": "2"
  }
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "entries": [
      {
        "cursor": "3",
        "level": "info",
        "event": "runtime_command",
        "message": "live command completed",
        "data": {
          "command": "focus.get"
        }
      }
    ],
    "next_cursor": "3"
  }
}
```

`logs.read` is intentionally scoped to structured QA/runtime entries. Raw engine stdout/stderr remains on disk in the session-level `logs.txt`.

### `record.export`

Request:

```json
{
  "command": "record.export",
  "params": {}
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "events": [
      {
        "seq": 1,
        "frame": 3,
        "type": "click",
        "target": "qa:start_button",
        "resolved": "qa:start_button"
      }
    ],
    "unconverted_events": []
  }
}
```

`record.export` is only available for recorder-enabled live sessions. It returns the raw runtime-derived trace that the CLI/library turns into a replayable candidate scenario plus review metadata.

### `edit.set_property`

Request:

```json
{
  "command": "edit.set_property",
  "params": {
    "target": "qa:settings_title",
    "property": "text",
    "value_input": "Patched Title"
  }
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "sequence": 1,
    "op": "set_property",
    "target": "qa:settings_title",
    "resolved": "path:/root/SettingsMenu/SettingsPanel/TitleLabel",
    "scene": "res://ui/settings_menu.tscn",
    "property": "text",
    "before": "Settings",
    "after": "Patched Title",
    "exportable": true
  }
}
```

### `edit.call_method`

Request:

```json
{
  "command": "edit.call_method",
  "params": {
    "target": "qa:apply_button",
    "method": "grab_focus"
  }
}
```

Success `data` mirrors the live-edit ledger entry and marks `exportable: false`.

### `edit.diff`

Request:

```json
{
  "command": "edit.diff",
  "params": {}
}
```

Success `data.operations` is the append-only live-edit ledger for the current session.

### `edit.export_patch`

Request:

```json
{
  "command": "edit.export_patch",
  "params": {}
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "format": "godot-qa.patch.v1",
    "scene": "res://ui/settings_menu.tscn",
    "operations": [
      {
        "op": "set_property",
        "target": "qa:settings_title",
        "property": "text",
        "value": "Patched Title"
      }
    ],
    "skipped_operations": [
      {
        "op": "call_method",
        "target": "qa:apply_button",
        "method": "grab_focus",
        "reason": "live_only"
      }
    ]
  }
}
```

`edit.export_patch` returns patch content; the CLI writes the file and reports the output path. If no exportable operations remain, the bridge returns `type: "patch_empty"`.

## Runtime/bridge surface after S16

### Implemented now

#### Session

```text
session.status
session.pause
session.resume
session.step
session.wait_frames
session.set_time_scale
session.reload_scene
session.profile
session.stop
```

#### Input

```text
input.tap_action
input.hold_action
input.release_action
input.click_target
input.mouse_click
input.mouse_wheel
input.touch_down
input.touch_move
input.touch_up
input.touch_tap
input.drag
input.key
input.text
input.joy_button
input.joy_motion
input.joy_state
```

`input.joy_button` accepts `button`, optional `device`, and optional `frames`. Supported buttons use the same named button aliases as `godot-qa input joy button`. The runtime emits an `InputEventJoypadButton` press for the requested device, waits `frames`, emits release, then waits one delivery frame. `device` and `frames` must be non-negative integers and default to `0` and `1`.

`input.joy_motion` accepts `axis`, `value`, optional `device`, and optional `frames`. Supported canonical axes are `left_x`, `left_y`, `right_x`, `right_y`, `trigger_left`, and `trigger_right`, with aliases for `left_trigger`, `right_trigger`, `lt`, `rt`, and Godot constant-style axis names. `value` must be numeric within `-1.0..=1.0`; the runtime emits an `InputEventJoypadMotion`, waits `frames`, and leaves the axis value active until a later motion command resets it.

`input.joy_state` is open-loop batch joypad input for concurrent multi-device holds. It accepts a non-empty `inputs` list plus optional non-negative integer `frames` (default `1`). Each `inputs` entry is `{device, motion|button}`: a non-negative integer `device` plus exactly one of `motion` (`{axis, value}`, the same named axes and `-1.0..=1.0` range as `input.joy_motion`) or `button` (`{name, pressed}`, the same named buttons as `input.joy_button` plus a boolean `pressed`). This is the same per-entry shape `policy.drive_tick` validates, but there is **no device allowlist** — any non-negative device is accepted. The whole list is validated before anything is emitted, so a malformed entry fails with `invalid_params` and injects nothing; the runtime then emits every entry back-to-back through the same `InputEventJoypadButton`/`InputEventJoypadMotion` paths with no frame await between them (so all devices' inputs land on one frame) and finally waits `frames` (`0` = emit and continue same-frame). Buttons stay pressed and axes stay latched until a later command changes them, and are released by the same `input.reset`/finalize/`session.change_scene` cleanup that covers `input.joy_button`/`input.joy_motion`. Success `data` carries `device` (`"joypad"`), `inputs_applied`, the normalized `inputs` list, `frames`, and `focus_owner`.

`input.mouse_wheel` accepts `x`, `y`, optional `dy`, and optional `dx`. Coordinates may be integer or float and must lie inside `viewport_rect()`. `dy`/`dx` default to `0.0` and at least one must be non-zero; the sign convention is web-style (positive `dy` is `wheel_down`, negative is `wheel_up`; positive `dx` is `wheel_right`, negative is `wheel_left`). Each axis emits `max(1, ceil(|delta|))` `InputEventMouseButton` press/release pairs with `factor = |delta| / pairs` and `button_mask` `0`, waiting one delivery frame per pair; the vertical axis is emitted before the horizontal axis. Bad coordinates or a zero delta fail with `invalid_params`.

`input.touch_down`, `input.touch_move`, `input.touch_up`, and `input.touch_tap` inject `InputEventScreenTouch`/`InputEventScreenDrag` events for a single held finger per `index` (default `0`). Coordinates may be integer or float and must lie inside `viewport_rect()`; `event.position` is set raw with no extra transform (`InputEventScreenTouch`/`InputEventScreenDrag` carry only `position`, no `global_position`). `input.touch_down` accepts `x`, `y`, and optional `index`; it emits a pressed touch, waits one frame, and fails with `touch_index_held` if that finger is already down. `input.touch_move` accepts `x`, `y`, optional `index`, and optional `frames` (default `1`, capped at `MAX_INPUT_GESTURE_FRAMES` = `1800`; larger fails with `invalid_params`); it interpolates a slide from the finger's last tracked position to the target over `max(frames, 1)` `InputEventScreenDrag` steps (the last step lands exactly on the target), setting `relative` and `velocity = relative / frame_delta` per step (frame delta uses `_root_window.get_process_delta_time()` with a `1/60` fallback), waits one frame per step, and fails with `touch_index_not_held` if the finger is not down. `input.touch_up` accepts optional `x`, `y`, and optional `index`; when `x`/`y` are omitted (the CLI sends explicit JSON `null`) it lifts at the finger's last tracked position, otherwise at the supplied coordinates, and fails with `touch_index_not_held` if the finger is not down. `input.touch_tap` accepts `x`, `y`, and optional `index`; it emits a press then a release (one frame each), fails with `touch_index_held` if the finger is already down, and adds `target`/`resolved` locator data when a visible control is hit. All four coerce `index` as a non-negative integer and reject out-of-bounds coordinates with `invalid_params`. Held fingers are lifted (normal `pressed=false` release, not a cancel) by `input.reset`, scenario finalize, and `session.change_scene`. On the first touch injection per process, if `input_devices/pointing/emulate_mouse_from_touch` is enabled the runtime logs a one-time informational note that injected touch also synthesizes mouse events; it never mutates the setting.

`input.drag` is a convenience gesture that presses at (`from_x`, `from_y`), interpolates to (`to_x`, `to_y`) over `max(frames, 1)` steps (`pos_i = from.lerp(to, i / steps)`, the final step forced exactly onto the target), then releases. Coordinates may be integer or float; both endpoints must lie inside `viewport_rect()` or the command fails with `invalid_params` (the failing endpoint is reported in `actual`). `frames` defaults to `8`, must be at least `1`, and is capped at `MAX_INPUT_GESTURE_FRAMES` = `1800` (below or above fails with `invalid_params`). `index` is coerced as a non-negative integer (default `0`) and applies only to the touch pointer. `pointer` (default `"touch"`) must be `"touch"` or `"mouse"`. The touch pointer rejects an already-held `index` with `touch_index_held`, then emits an `InputEventScreenTouch` press, one `InputEventScreenDrag` per step (each carrying `relative` and `velocity = relative / frame_delta`), and a release, waiting one frame after each event. The mouse pointer emits an initial `InputEventMouseMotion`, a left `InputEventMouseButton` press, one `InputEventMouseMotion` per step carrying `relative`, `velocity`, and `button_mask = MOUSE_BUTTON_MASK_LEFT` (the plain motion emitter sets no mask, so Control drag detection needs this masked variant), and a left button release, again waiting one frame per event. Success `data` carries `device` (`"touch"` or `"mouse"`), `pointer`, `from`, `to`, `frames`, and `steps` (`max(frames, 1)`), plus `index` for the touch pointer only; the mouse path never touches held-touch state.

`input.key` accepts a single `key` string using the same named-key vocabulary as `godot-qa input key` (`Enter`, `Escape`, `Space`, `Tab`, `Backspace`, `Up`, `Down`, `Left`, `Right`, `0-9`, `A-Z`, plus aliases such as `Return`, `Esc`, and `ArrowUp`/`ArrowDown`/`ArrowLeft`/`ArrowRight`). The runtime emits an `InputEventKey` press, waits one frame, emits release, then waits one frame. Printable keys carry the matching `unicode` codepoint (`Space`=32, digits 48-57, letters lowercase 97-122) so a focused text control inserts the character; letters emit lowercase unicode with no Shift synthesis, and control keys keep `unicode` `0`. An unknown key fails with `invalid_params`. Success `data` carries `device`, `key`, the additive `unicode` field (the emitted codepoint, or `0` for control keys), and `focus_owner`. Engine caveat: `Enter` submits a focused `LineEdit` and ends its editing state (default `keep_editing_on_text_submit`), so subsequent key/text input is dropped by that control until it is re-activated (for example via `input.click_target`).

`input.text` accepts a non-empty `text` string (at most `900` characters; empty or over-length fails with `invalid_params`). The runtime emits one `InputEventKey` press/release pair per codepoint of `text`, setting `unicode` from the codepoint on both press and release and a best-effort keycode (`OS.find_keycode_from_string` on the uppercased character, `KEY_NONE` fallback), waiting one frame after each event (2 frames per character against the `MAX_INPUT_GESTURE_FRAMES` = `1800` cap). Text entry is case-sensitive and supports punctuation and non-ASCII characters, and no modifiers (Shift/Ctrl/Alt) are synthesized. Success `data` carries `device`, `text`, `characters`, and `focus_owner`.

#### Saved scenario setup

```text
setup.teleport_node
fixture.teleport_to_target
setup.spawn_scene
setup.set_property
setup.call_method
fixture.call
setup.joy_connect
setup.joy_disconnect
```

These commands are used by saved scenario steps and live setup scripts to establish deterministic fixture state. `setup.teleport_node` and `fixture.teleport_to_target` reset transient physics velocity by default after placement; pass `preserve_velocity: true` only when continuing existing motion is the behavior under test. `setup.call_method` is intentionally limited to methods beginning with `qa_`. `fixture.call` addresses registered fixture drivers by `driver` and `action`; it is for deterministic setup/mutation only and is not callable through assertions or read-only state paths.

`setup.joy_connect` and `setup.joy_disconnect` back the saved `joy_connect`/`joy_disconnect` scenario steps and simulate joypad *hardware connection* state (distinct from `input.joy_button`/`input.joy_motion`, which inject events regardless of connection state). Both fire the engine's `Input.joy_connection_changed(device, connected)` **signal** via `Input.joy_connection_changed.emit(...)` — the same signal the platform layer emits on a real connect/disconnect — so a local-multiplayer lobby's signal-driven connect/disconnect handlers run under the tool. Both accept `devices`: a required non-empty list of unique non-negative integers (deduped/normalized runtime-side). `setup.joy_connect` also accepts an optional non-empty `name` (default `"QA Virtual Pad"`), echoed as QA metadata; it fires `connected == true` once per device and is idempotent (a device already announced is not re-emitted, so a listener cannot double-join). `setup.joy_disconnect` fires `connected == false` per device but only for devices previously faked via `setup.joy_connect`: a request naming any non-faked device fails with `invalid_params` (`actual` lists the offending devices, `faked_devices` the current fake set) and fires nothing. Connect also rejects a currently real connected device ID; disconnect rejects one that has become real. The QA roster is updated before each signal, so a callback that polls `effective_connected_joypads()` sees the new state. Disconnect releases injected button and axis state for those devices before notifying listeners, leaving other devices' input alone. Each command waits one delivery frame after the batch and returns `data` carrying `devices` (the normalized list acted on), `announced_joypads` (the sorted set of QA-announced-connected devices — the field to assert on), `connected_joypads` (the real `Input.get_connected_joypads()` roster, for transparency), and — for connect — the resolved `name`. The QA-announced set is process-global tracking that survives a scene reload, so a matching disconnect signal is fired and the set cleared by `input.reset`, scenario finalize, `session.change_scene`, and live `session.stop`, keeping fakes from leaking into the next warm scenario.

**Engine roster limitation:** `Input.joy_connection_changed` is a signal, not a bound method — the roster-mutating C++ method `Input::joy_connection_changed(device, connected, name, guid)` is not exposed to GDScript (`Input.has_method("joy_connection_changed")` is `false`) and the signal carries only `(device, connected)`. So these commands fire the signal (verified to invoke connected handlers under `--headless`) but do **not** add the device to `Input.get_connected_joypads()` and do not change `Input.get_joy_name(device)`. `name` is accepted and echoed for contract stability and forward compatibility if a future engine binds the four-argument method.

For lobbies that poll the roster rather than listening to the signal, the addon ships a game-facing shim in the same family as `QaArgs.effective_user_args()` (the warm-session-aware replacement for `OS.get_cmdline_user_args()`): `QaRuntime.effective_connected_joypads() -> Array[int]` is a static read-only helper returning the sorted, deduped union of the real `Input.get_connected_joypads()` and the devices currently faked via `setup.joy_connect`. With no fakes it returns exactly the engine roster, so a game can adopt it unconditionally as a one-line replacement at the read site. Pure `Input.get_connected_joypads()` polling without the shim remains unvalidatable on this engine version.

Live `session.change_scene` and `session.reload_scene` reset injected input before replacing the scene and return `input_cleanup_failed` if that reset reports an error. Live `session.stop` also resets injected input before requesting process exit and includes `input_cleanup_failures`, `joypad_roster_before`, and `joypad_roster_after` in its reply. Synthetic controller checks still need separate physical-device testing before claiming hardware acceptance.

#### Policy-driven input

```text
policy.drive_tick
policy.release_inputs
```

These commands back the saved `drive_policy` scenario step. `policy.drive_tick` accepts `policy`, `devices`, optional `params`, and the loop counters `tick`/`frame`; it calls the registered policy provider (or a `builtin:` policy) with a read-only request dictionary, validates the returned `{"inputs": [...]}` list, then emits each entry through the same `InputEventJoypadMotion`/`InputEventJoypadButton` paths as `input.joy_motion` and `input.joy_button`, restricted to the allowed `devices`. The whole list is validated before anything is emitted, so a malformed policy result injects no input. It never awaits frames (the scenario runner owns pacing) and never mutates node state. `policy.release_inputs` zeroes touched axes and releases pressed buttons for exactly the given `devices`; the scenario runner calls it whenever a `drive_policy` step stops.

Policy providers register through `QaRuntime.register_policy_provider(id, owner, method, metadata)`, mirroring fixture drivers with an arity-1 method, but they are read-only consumers: mutating method names are rejected exactly like state providers, and the policy computes desired input instead of changing state.

#### Live edit

```text
edit.set_property
edit.call_method
edit.diff
edit.export_patch
```

#### Inspection

```text
tree.dump
ui.dump
focus.get
logs.read
```

#### Snapshot

```text
snapshot.capture
screenshot.capture
```

`screenshot.capture` accepts optional `allow_fallback` (default `true` for compatibility). With `false`, an unavailable real viewport image fails `screenshot_capture_failed` rather than writing Godot's diagnostic fallback PNG. Successful responses include `capture_source: "godot_viewport"`, `draw_forced: true`, and the effective `allow_fallback`. The isolated `screenshot assert-startup-presented` CLI command uses `false` only after it has saved the compositor PNG; its compositor capture is CLI-owned and never a bridge command.

#### Assertions

The bridge now dispatches these assertion commands through the same runtime layer used by `qa_runner.gd`, so live bridge callers and one-shot scenarios share locator resolution and mismatch types:

```text
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
assert.state_matches
assert.surface_probe
assert.surface_matches
assert.visible_pixels
assert.not_occluded
assert.renders_above
assert.renders_below
```

`assert.state_matches` reads runtime state and compares JSON-compatible values. With `source: "provider"`, it reads a registered state provider by explicit provider ID and does not require `target`. With `source: "method"`, it reads a resolved target method whose name matches exactly one approved read-only QA state pattern: `qa_*`, `get_qa_*`, or `get_*_qa_state`; this is not an arbitrary method-call escape hatch. Registered provider methods do not need `qa_` naming because explicit registration is the permission boundary; they must still be read-only, zero-argument, JSON-compatible, and non-mutating by behavior. State checks support `equals`, `contains`, `present`, `absent`, `approx_equals`, `approximately_equals`, `greater_than`, and `less_than`; `approximately_equals` is an alias for `approx_equals`, and both accept optional `epsilon` with default `0.0001`. Exact numeric `equals` mismatches include additive `actual_full_precision`, `expected_full_precision`, and `numeric_delta` fields when both compared values are numbers.

Provider state request:

```json
{
  "command": "assert.state_matches",
  "params": {
    "source": "provider",
    "provider": "race_feel",
    "matches": [
      {"path": "phase", "operator": "equals", "expected": "gameplay"}
    ]
  }
}
```

Provider state success data includes `source: "provider"`, `provider`, `method`, and `matched`.

Method-source failures identify the rejected method, target, allowed patterns, and a rejected reason:

- `name`: the method name does not match `qa_*`, `get_qa_*`, or `get_*_qa_state`.
- `arguments`: the method exists but requires arguments.
- `missing_method`: the target does not expose the requested method.
- `non_json_return`: the method returned a Godot object, resource, callable, signal, or other non-JSON-compatible value.
- `runtime_error`: calling the method recorded a runtime/script error.

qa_state.read
-------------

`qa_state.read` is the token-protected live bridge command used by `qa-state validate` to read a registered state provider or a single legacy QA state method result before CLI-side schema validation. The bridge command is intentionally read-only and does not evaluate schemas, write files, mutate gameplay state, add debug labels, or infer state from hidden text.

Request params:

```json
{
  "command": "qa_state.read",
  "token": "<session-token>",
  "params": {
    "target": "qa:race",
    "method": "get_qa_race_state"
  }
}
```

Provider request params:

```json
{
  "command": "qa_state.read",
  "token": "<session-token>",
  "params": {
    "provider": "race_feel"
  }
}
```

`target` is resolved through the shared runtime target resolver. `method` must match the same allowlist used by `assert.state_matches`: `qa_*`, `get_qa_*`, or `get_*_qa_state`. The method must require zero arguments, be read-only by convention, return JSON-compatible dictionaries, arrays, strings, numbers, booleans, or null, and complete without runtime errors.

Successful responses include the resolved target, method, and returned state:

```json
{
  "protocol_version": "0.1",
  "ok": true,
  "data": {
    "target": "qa:race",
    "resolved": "/root/Race",
    "method": "get_qa_race_state",
    "state": {"phase": "racing", "laps": 3}
  }
}
```

Rejected requests use the normal protocol error envelope and preserve the state-method safety model. Failure data identifies the method name, target, allowed patterns, and rejection reason. Rejection classes include `name` for non-allowlisted names, `missing_method` for missing providers, `arguments` for methods that require parameters, `runtime_error` for script/runtime errors during the call, and `non_json_return` for Godot objects or other non-JSON-compatible return values. Invalid or missing session tokens are rejected before dispatch by the live transport.

state.record
------------

`state.record` reads a state value through the exact same read-only state-source path as `assert.state_matches` and `wait_until` (`_resolve_state_source`), but records the value as run evidence instead of asserting on it. It is the runtime backing for the scenario `record_state` step. Like `qa_state.read`, it is read-only and never writes files, mutates gameplay state, or evaluates schemas.

Request params:

```json
{
  "command": "state.record",
  "token": "<session-token>",
  "params": {
    "name": "match_summary",
    "source": "provider",
    "provider": "match_state",
    "path": "summary"
  }
}
```

Method-source request params read a resolved target method instead:

```json
{
  "command": "state.record",
  "token": "<session-token>",
  "params": {
    "name": "match_summary",
    "source": "method",
    "target": "qa:game",
    "method": "get_qa_match_state"
  }
}
```

`name` is required and non-empty. `source: "provider"` reads a registered state provider by id; `source: "method"` reads a resolved `target` method matching the same allowlist as `assert.state_matches` (`qa_*`, `get_qa_*`, or `get_*_qa_state`, zero-argument, read-only, JSON-compatible). Optional `path` is a dot-separated selection into the read value; an absent path segment records `null` with `present: false`. Successful responses echo the record under `data`:

```json
{
  "protocol_version": "0.1",
  "ok": true,
  "data": {
    "name": "match_summary",
    "source": "provider",
    "provider": "match_state",
    "path": "summary",
    "present": true,
    "value": {"winner": "team_a", "score": [3, 1]},
    "value_bytes": 41
  }
}
```

Method-source responses additionally carry `target`, `resolved`, and `method`. The scenario runner stamps `step_index` and keys these entries by `name` into the top-level `state_records` map in `result.json`. Unresolved providers/targets, invalid/mutating/argument-taking methods, runtime errors during the call, and non-JSON-compatible values are rejected with the same error types the `state_matches`/`wait_until` state-source path produces. The recorded value is bounded: its serialized form must stay within a fixed byte cap, and an over-cap value fails with `type: "state_record_value_too_large"` (carrying `expected.max_value_bytes` and `actual.value_bytes`) rather than being silently truncated. A missing `name` fails with `type: "state_record_invalid_args"`.

Visual assertion requests are screenshot-backed deterministic 2D/UI checks and use these runtime commands:

- `assert.visible_pixels` with `target`, `snapshot`, and at least one threshold (`min_ratio` and/or `min_count`)
- saved-snapshot pixel evaluation with `target`, `snapshot`, and at least one threshold (`min_ratio` and/or `min_count`) is CLI/scenario-owned and reads saved snapshot artifacts; it uses the same runtime-captured snapshot evidence but does not require a live target-hidden capture
- `assert.not_occluded` with `target`, `snapshot`, and optional `occluder`
- `assert.renders_above` with `target`, `other`, and `snapshot`
- `assert.renders_below` with `target`, `other`, and `snapshot`

The request fields above are required unless noted optional. Runtime visual assertions must evaluate real rendered viewport pixels from the named `snapshot`; scene-tree `visible`/`hidden` state is not enough evidence for these commands.

The CLI `visual preflight --json` payload reports launch-level visual support before a runtime command is attempted. `data.visual_support` contains `real_viewport_pixels`, `target_hidden_capture`, `mask_generation`, and `saved_snapshot_object_pixels`; each capability includes `available`, `reason`, `expected`, `actual`, and `remediation`. `target_hidden_capture`, `mask_generation`, and saved-snapshot object pixels are reported as available only when real viewport pixels are available for the selected launch mode. Target-specific limits, such as unsupported 3D targets, missing `qa_visual_rect`, missing `qa_set_visual_visible`, or unsupported coordinate spaces, are still resolved during the actual assertion.

Minimum failure evidence payload for visual assertions:

- `command`
- `snapshot`
- `target` (`other` for order assertions, `occluder` when known for occlusion failures)
- `artifacts.screenshot` under `.qa/artifacts/...`
- diagnostic artifacts when produced, such as `artifacts.original`, `artifacts.target_hidden`, `artifacts.mask`, and `artifacts.overlay`
- `target_rect` or `projected_rect`
- measured `visible_pixel_count` and/or `visible_pixel_ratio`
- configured threshold fields (`min_count` and/or `min_ratio`) when applicable
- failure-specific `expected`, `actual`, and remediation guidance when geometry, coordinate conversion, or screenshot capture is unsupported

Target rects are reported in the 2D canvas coordinate space, but screenshots are captured at the OS window resolution. Under content scaling (`display/window/stretch/mode`) these differ, so `assert.visible_pixels` converts the canvas rect into screenshot space with `Window.get_final_transform()` before sampling pixels. Success and failure `error.data` include the canvas-space `target_rect` (unchanged) plus additive `target_rect_screenshot` (the transformed rect actually sampled), `canvas_to_screenshot` (`{scale: [sx, sy], offset: [ox, oy]}`), and `screenshot_size`. When the window matches the canvas the transform is the identity and `target_rect_screenshot` equals `target_rect`. The changed-pixel ratio denominator is the clipped screenshot-space rect area, so numerator and denominator stay in the same space.

Unsupported or ambiguous visual evidence must fail explicitly. Stable failure classes include `ambiguous_target`, `unsupported_visual_assertion`, `unsupported_coordinate_space`, `screenshot_capture_failed`, `screenshot_geometry_mismatch`, `headless_screenshot_unsupported`, and more specific runtime limitation errors. `screenshot_geometry_mismatch` fires when the transformed full-canvas rect does not match the captured screenshot size (an unforeseen capture/scaling mismatch) and carries `expected`/`actual` `screenshot_size`, the `canvas_to_screenshot` transform, the `canvas_rect`, and remediation guidance rather than silently sampling the wrong pixels. `unsupported_visual_assertion` identifies the missing evidence capability, such as viewport rect extraction, visibility control for target-hidden captures, screenshot capture, or coordinate conversion. `unsupported_coordinate_space` includes the assertion command, target, requested coordinate space, actual coordinate space when known, and remediation guidance. `screenshot_capture_failed` includes display/screenshot details and should tell projects to rerun with a real display or supported virtual display rather than relying on synthetic fallback images. Visual diagnostics such as masks/overlays are candidate artifacts only; bridge/runtime responses must not imply baseline approval and must not write to `qa/baselines`.

Targets may expose explicit viewport-space visual evidence through `qa_visual_rect()` or `qa_visual_rect` metadata returning `{x, y, width, height}`, `[x, y, width, height]`, or `Rect2`. Non-`CanvasItem` targets must also expose `qa_set_visual_visible(visible)` so screenshot-backed assertions can capture target-hidden delta evidence without inferring arbitrary pixels. `Line2D` targets use viewport bounds of the rendered stroke, including width and transformed points; a zero-length or non-rendered line must not satisfy a visible-pixel assertion.

`not_occluded`, `renders_above`, and `renders_below` require runtime evidence that can compare rendered pixels against target-hidden or related-object captures. Saved-snapshot object-pixel checks require a previously captured real screenshot plus snapshot node `visual_evidence` for the target; they prove that the object contributed useful pixels in that screenshot, but they do not prove current occlusion or draw order after the snapshot was saved.

### `assert.visible_pixels`

Request:

```json
{
  "command": "assert.visible_pixels",
  "params": {
    "target": "qa:important_actor",
    "snapshot": "occluded",
    "min_ratio": 0.25
  }
}
```

Failure shape (example):

```json
{
  "ok": false,
  "error": {
    "type": "visible_pixel_ratio_below_threshold",
    "message": "visible pixel ratio below configured threshold",
    "data": {
      "command": "assert.visible_pixels",
      "target": "qa:important_actor",
      "snapshot": "occluded",
      "target_rect": {"x": 640, "y": 360, "width": 160, "height": 160},
      "target_rect_screenshot": {"x": 320, "y": 180, "width": 80, "height": 80},
      "canvas_to_screenshot": {"scale": [0.5, 0.5], "offset": [0, 0]},
      "screenshot_size": [960, 540],
      "visible_pixel_count": 1200,
      "visible_pixel_ratio": 0.0468,
      "expected": {"min_ratio": 0.25},
      "actual": {"visible_pixel_ratio": 0.0468},
      "artifacts": {
        "screenshot": "/path/to/project/.qa/artifacts/.../snapshots/occluded/screenshot.png",
        "visual_mask": "/path/to/project/.qa/artifacts/.../diagnostics/important_actor-mask.png"
      }
    }
  }
}
```

### `assert.not_occluded`

Request:

```json
{
  "command": "assert.not_occluded",
  "params": {
    "target": "qa:important_actor",
    "snapshot": "occluded",
    "occluder": "qa:debug_overlay"
  }
}
```

Failure `error.data` includes `command`, `target`, `snapshot`, runtime geometry (`target_rect` or `projected_rect`), pixel evidence (`visible_pixel_count`, `visible_pixel_ratio`), and `occluder` when identified.

### `assert.renders_above`

Request:

```json
{
  "command": "assert.renders_above",
  "params": {
    "target": "qa:hud_label",
    "other": "qa:hud_backplate",
    "snapshot": "hud"
  }
}
```

Failure `error.data` includes `command`, `target`, `other`, `snapshot`, runtime geometry (`target_rect`/`projected_rect` and `other_rect`/`other_projected_rect` when available), and ordering evidence derived from the screenshot-backed runtime analysis.

### `assert.renders_below`

Request:

```json
{
  "command": "assert.renders_below",
  "params": {
    "target": "qa:hud_backplate",
    "other": "qa:hud_label",
    "snapshot": "hud"
  }
}
```

Failure `error.data` matches `assert.renders_above` fields, with inverted ordering expectation.

### `screenshot.capture`

Request:

```json
{
  "command": "screenshot.capture",
  "params": {
    "name": "main_menu"
  }
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "name": "main_menu",
    "artifacts": {
      "screenshot": "/path/to/project/.qa/artifacts/live/20260422T120000Z/screenshots/main_menu.png"
    }
  }
}
```

The bridge sanitizes `params.name` and always writes under the controlled session `screenshots_dir`. The CLI is responsible for any user-requested copy destination, screenshot quality checks, and all baseline/diff logic. If Godot stages a transparent or otherwise low-information PNG, CLI screenshot commands fail it as `screenshot_low_information`.

`screenshot sequence` has no dedicated bridge command: the CLI drives the capture loop by alternating `screenshot.capture` and `session.wait_frames`. Its `--stop-after-state` predicate is therefore evaluated CLI-side by polling `assert.state_matches` once per captured frame; when the predicate matches the CLI stops the loop early (the manifest holds fewer frames) and echoes `stop_after_state`, `stop_condition_matched`, and `stop_condition_timed_out` in the result. When the predicate never matches before the plan completes, the CLI reports `screenshot_sequence.stop_condition_timeout`. The predicate uses the same read-only `state_matches` shape as `audio.capture` and `trace.record`.

### `audio.capture`

Request:

```json
{
  "command": "audio.capture",
  "params": {
    "name": "current_race",
    "seconds": 3.0,
    "max_seconds": 60.0,
    "bus": "Master",
    "stop_after_state": {
      "target": "qa:race",
      "source": "method",
      "method": "get_qa_victory_state",
      "matches": [{"path": "victory_elapsed_seconds", "operator": "greater_than", "expected": 10}],
      "sample_every_frames": 1
    },
    "audio_dir": "/path/to/project/.qa/artifacts/live/20260422T120000Z/audio",
    "wav_path": "/path/to/project/.qa/artifacts/live/20260422T120000Z/audio/current_race.wav",
    "max_bytes": 10485760
  }
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "name": "current_race",
    "bus": "Master",
    "requested_seconds": 3.0,
    "effective_seconds": 3.0,
    "max_seconds": 60.0,
    "host_microphone_capture": false,
    "stop_after_state": {
      "matched": true,
      "sample_count": 42,
      "sample_every_frames": 1
    },
    "stop_condition_matched": true,
    "stop_condition_timed_out": false,
    "artifacts": {
      "wav": "/path/to/project/.qa/artifacts/live/20260422T120000Z/audio/current_race.wav"
    },
    "warnings": []
  }
}
```

The session bridge injects `audio_dir`, `wav_path`, and `max_bytes`; clients do not choose arbitrary runtime write paths. `stop_after_state` is optional and uses the same read-only `state_matches` predicate shape as assertions. When present, the runtime samples the predicate every `sample_every_frames` process frames, stops capture early on match, and otherwise writes the bounded WAV before the CLI reports `audio_capture.stop_condition_timeout`. The runtime validates that `wav_path` is under `audio_dir`, creates only the controlled audio directory, records no host microphone input, and returns structured failure kinds such as `audio_capture.unavailable`, `audio_capture.failed`, `audio_capture.duration_too_long`, `audio_capture.stop_condition_timeout`, or `audio_capture.bus_not_found` when capture cannot produce passing evidence. Authenticated live command token checks and token redaction rules are the same as other session commands.

### `audio.export`

Request:

```json
{
  "command": "audio.export",
  "params": {
    "name": "exported_sfx",
    "target": "qa:audio_controller",
    "method": "qa_export_audio_track",
    "args": ["sfx_bus_snapshot"],
    "audio_dir": "/path/to/project/.qa/artifacts/live/20260422T120000Z/audio",
    "wav_path": "/path/to/project/.qa/artifacts/live/20260422T120000Z/audio/exported_sfx.wav",
    "max_bytes": 10485760
  }
}
```

Success:

```json
{
  "ok": true,
  "protocol_version": "0.1",
  "data": {
    "name": "exported_sfx",
    "target": "qa:audio_controller",
    "method": "qa_export_audio_track",
    "artifacts": {
      "wav": "/path/to/project/.qa/artifacts/live/20260422T120000Z/audio/exported_sfx.wav"
    },
    "warnings": []
  }
}
```

`audio.export` is for project-owned fallback evidence when bus capture is unavailable or not precise enough. The target must resolve through the live target resolver, and the method must begin with `qa_`. The hook may return bytes/base64 for the bridge to accept under the controlled `wav_path`, or a JSON-compatible dictionary that points to a WAV already under `audio_dir`. Structured rejections include `audio_export_rejected`, `audio_export_path_rejected`, `audio_export_invalid_metadata`, `audio_export_oversized`, `audio_export_invalid_return`, and `audio_export_failed`, with command, target, method, expected state, actual state, and artifact context when available.

### `audio.render`

Request:

```json
{
  "command": "audio.render",
  "params": {
    "name": "music_bed",
    "target": "qa:audio_controller",
    "method": "qa_fill_audio",
    "args": [],
    "duration_seconds": 8.0,
    "effective_duration_seconds": 8.0,
    "max_seconds": 60.0,
    "mix_rate": 44100,
    "audio_dir": "/path/to/project/.qa/artifacts/live/20260422T120000Z/audio",
    "wav_path": "/path/to/project/.qa/artifacts/live/20260422T120000Z/audio/music_bed.wav",
    "max_bytes": 10485760
  }
}
```

`audio.render` is the offline-bake counterpart to `audio.export`: the runtime owns the bake loop and the WAV container, while the project supplies only a `qa_` voice-fill method. The runtime computes `frame_count = ceil(effective_duration_seconds × mix_rate)`, calls `method(frame_count, mix_rate, ...args)`, and packs the returned mono samples (`PackedFloat32Array`, downmixed `PackedVector2Array`, or `Array` of floats in `[-1, 1]`) into a controlled 16-bit WAV under `wav_path`. It works headless because audio is synthesized to a buffer rather than captured from an output device. The method must begin with `qa_` and resolve through the live target resolver. Structured rejections include `audio_render.rejected`, `audio_render.oversized`, `audio_render.invalid_return`, and `audio_render.failed`, with command, target, method, expected state, actual state, and artifact context when available. The bridge injects the controlled `audio_dir`, `wav_path`, and `max_bytes` exactly as for `audio.capture`/`audio.export`.

### Notes

Remaining deferred surfaces should keep returning structured `unsupported_command` or CLI `not_implemented` until their spec work lands.

## Scenario Audio Artifact Security

Saved-scenario `audio_capture` and `audio_export` are runner-owned artifact operations; live `audio.capture` and `audio.export` are authenticated session operations with the same controlled path boundary. The CLI prepares the per-run or per-session artifact root and passes a controlled `audio_dir` plus intended `wav_path` to the Godot runner or live bridge. Runtime export hooks may only return bytes/base64 for the runner or bridge to write, or a JSON-compatible dictionary that points to a WAV already written inside that controlled `audio_dir`.

The bridge/runtime must reject any audio export dictionary whose path escapes the controlled audio directory, including absolute paths, `res://` project paths, `user://` paths outside the run root, and path traversal. It must not expose an arbitrary destination path parameter to live bridge clients or scenarios. Copying or publishing an artifact outside `.qa/artifacts` is CLI-side behavior, not a Godot bridge write capability.

Audio export hooks are restricted to resolved QA targets and methods beginning with `qa_`. Rejections are structured (`audio_export_rejected`, `audio_export_path_rejected`, `audio_export_invalid_metadata`, `audio_export_oversized`, or `audio_export_invalid_return`) and include the command, target, method when applicable, expected state, actual state, and artifact context.

## Target identifiers

Supported target formats:

```text
qa:<id>                  Stable test id from metadata or documented QA groups
path:/root/Main/Button   Absolute NodePath
name:StartButton         Fallback name search, ambiguous if multiple
```

Prefer `qa:<id>` in all scenarios and agent commands. The canonical group form for `qa:<id>` is `qa.id.<id>`. `qa.<id>` may be accepted as a compatibility alias, but metadata `qa_id` is preferred because it travels with the node and keeps the lookup contract explicit.

## Android bootstrap and transport

The desktop runner and Android bootstrap use the same serialized authenticated
TCP request host. It binds to `127.0.0.1` on both targets; Android access is
provided only through the session's exact ADB forward.

The bridge checks the fixed
`user://godot_qa/android_live_bootstrap.json` file only when all three conditions
hold: the runtime has the `android` feature, is a template build, and
`OS.is_debug_build()` is true. Desktop builds, editor runs, release templates,
missing or malformed files, and absent explicit bootstrap remain inactive.

Bootstrap schema version `1` requires a valid session ID, non-empty token,
loopback port, optional `res://` scene, and a JSON array of project arguments.
It may also carry an optional `mode` key: `live` (the default when the key is
absent) or `scenario`; any other value is an invalid bootstrap. The runtime
derives its device artifact root as `user://godot_qa/live/<session-id>` and does
not accept a host-provided device output root. It initializes arguments through
`QaArgs.effective_user_args()`, replaces the packaged main scene when requested,
waits for rendered frames, reports the actual viewport, starts the shared host,
and deletes the one-shot bootstrap.

In `live` mode the session keeps the packaged main scene. In `scenario` mode with
an empty bootstrap scene the runtime boots the packaged main scene for its frames
and then frees it (the Android analogue of the desktop warm sceneless boot), so a
per-scenario `QaScenarioExecutor` owns the scene lifecycle and injected input can
never reach the game's main scene.

Android live commands preserve the existing command names and response objects.
The host client translates controlled artifact path parameters into the private
device root, accepts returned paths only beneath that exact root, pulls bytes
with `adb exec-out run-as <package> cat`, writes host artifacts atomically, and
rewrites paths before existing consumers read them. Directory roots and path
traversal are rejected, as are `:` characters in relative suffix segments that
could become Windows drive-relative paths or alternate data streams. The token is not sent through the host-file RPC fallback
for Android.

`scenario.run` stays unsupported for `live`-mode Android sessions
(`android_command_unsupported`). For `scenario`-mode sessions it is accepted,
provided its `request_path` resolves beneath the session's artifacts root
(`user://godot_qa/live/<session-id>`); a request outside that root is rejected
with `invalid_params`.

Android one-shot `scenario run` uses a per-run device layout beneath that root at
`user://godot_qa/live/<session-id>/run`. The CLI pushes the rewritten request.json
to `.../run/request.json` (with `run.frame_budget` stripped and every artifact
path rewritten to the device root), pre-creates the `run/snapshots`,
`run/temporal`, and `run/audio` directories, and dispatches `scenario.run` with a
`request_path` pointing at the device request and a wall-clock `timeout_seconds`
(never `timeout_frames`). The response stays liveness-only (`scenario`, `ok`,
`write_ok`, `result_path`, `failure_count`); the authoritative result is the
`.../run/result.json` file, which the CLI pulls, rewrites to host paths, and
materializes claimed evidence from. `.../run/progress.jsonl` is pulled best-effort
and rewritten path-only.

## Offline asset-trial evidence collection

Asset trials use an offline QA runner that loads the source scene without
entering the scene tree or invoking `_ready`, applies only the manifest's native
texture/stream slots, and saves a candidate scene through Godot's scene APIs.
The trial runner is not a bridge command and does not add filesystem writes to
the live protocol. The ordinary validation scenario still runs through the
existing scenario runner. A private trial request selects existing snapshot,
audio-capture, and `record_state` steps; a runtime result advertises
`asset_trial_v1` only when the copied addon includes the scene writer, media
import probe, and scenario evidence collector. Collection reports facts from
Godot about the resolved node and resource, decoded PNG pixels or imported WAV
stream, visible pixels, playback overlap, render backend, and audio driver. It
does not supply expected pass/fail values or create a general-purpose resource
serialization interface.
