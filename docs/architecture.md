# Architecture

## Components

### 1. CLI/library

The CLI is the stable automation surface used by humans, CI, AI agents, and the MCP server.

Current implementation:

- The shipped CLI/library is the Rust Cargo workspace.
- The CLI contract, artifact layout, exit semantics, and Godot runtime boundaries are stable integration contracts.
- `godot-qa-cli` owns the parser/binary entrypoint, `godot-qa-core` owns shared result/config types, `godot-qa-process` owns process-backed `doctor`, `import`, bounded `check`, and session process lifecycle helpers, `godot-qa-runtime` owns live-session metadata plus localhost transport, `godot-qa-debug` owns the hot-reload debug-host listener and the minimal Godot-Variant remote-debugger codec, `godot-qa-scene` owns the pure `.tscn` parser and structural differ that back faithful scene live-editing, and `godot-qa-scenario` owns scenarios, validation orchestration, screenshots, recorder, live edit, and MCP-facing routing.

Workspace responsibilities:

- `crates/godot-qa-cli`
  - Owns the user-facing binary entrypoint, parser wiring, human/JSON output selection, and exit-code mapping.
- `crates/godot-qa-core`
  - Owns stable shared types such as `CommandResult`/`FailureDetail` equivalents, artifact-path models, config/project resolution types, and shared error categories.
- `crates/godot-qa-process`
  - Owns Godot discovery, version probing, process spawning, one-shot launch handling, import/check orchestration primitives, and process-owned artifact/log capture.
- `crates/godot-qa-runtime`
  - Owns live-session metadata, token handling, localhost transport, runtime request/response validation, and live command routing primitives.
- `crates/godot-qa-scenario`
  - Owns scenario parsing/execution orchestration, recorder shaping, screenshot/layout/assertion orchestration, live-edit orchestration, and MCP-facing command routing through the same CLI/library behavior.

Responsibilities:

- Discover Godot binary and project paths.
- Run one-shot scenarios.
- Start/stop live sessions.
- Send commands to the runtime bridge.
- Save artifacts.
- Compare screenshots.
- Print human-readable output and JSON output.
- Exit with correct status codes.
- Bundle the repo-local AI skill package and install it into project scope with `install-skills`.

Bundled AI skill packaging lives under `skills/godot-qa/` and is a repo-bundled integration surface layered on top of the same Rust CLI/library. It is not a second runtime implementation.

Scenario bundle orchestration remains CLI-owned. The CLI is the source of truth for bundle expansion, child ordering, strict pass/fail decisions, retry eligibility, and summary JSON, while `godot-qa-process` owns process launch details, display lifecycle metadata, owned-child cleanup status, and stale-process cleanup outcomes surfaced back into bundle attempts.

### 2. Godot addon/runtime bridge

The bridge runs inside Godot and has access to runtime truth.

Responsibilities:

- Load scenes.
- Wait frames.
- Simulate input.
- Inspect scene tree.
- Inspect UI Control state.
- Capture screenshots.
- Run assertions that require engine state.
- Return JSON-compatible data to the CLI.

Installed versus active behavior:

- Enabling the plugin registers the `GodotQaBridge` autoload singleton in the project.
- The bridge is still dormant until QA mode is explicitly enabled with `--qa`, `--qa-*`, or the `godot_qa/enabled` project setting.
- Projects can call `GodotQaBridge.is_runtime_qa_active()` to gate expensive pull-only QA state; editor-only helpers such as the Godot QA Profiler do not activate this runtime flag.
- An installed-but-inactive bridge should return structured `bridge_inactive` failures instead of pretending runtime QA is available.
- Editor-only helpers may write project-level artifacts without activating the runtime bridge. `qa_editor_profiler.gd` registers editor controls and captures raw editor debugger profiler messages into `.qa/artifacts/editor-profiler/latest.json`; `godot-qa profile editor export --json` only reads and validates that artifact.

### 3. Scenario runner

The scenario runner makes interactions reproducible.

Responsibilities:

- Parse scenario definitions.
- Configure viewport, locale, seed, and test mode.
- Execute steps deterministically.
- Save before/after snapshots.
- Evaluate assertions.
- Exit pass/fail.

The current shipped one-shot runner keeps run-directory ownership in Rust while Godot owns runtime truth for snapshot contents. `result.json.artifacts` stays flat and run-level, while `result.json.snapshot_artifacts` maps each snapshot name to `root`, `screenshot`, `tree`, `ui`, `focus`, and `errors`.

Runtime acceleration follows the same boundary. Default fastest frame pacing is CLI-owned launch metadata: for `scenario run` and manifest-backed `check --tests`, Rust may add generated `--fixed-fps <n>` and `--disable-vsync` flags and report `frame_pacing` evidence. The Godot addon owns the effective Engine state. The CLI validates requested `time_control` schema and bounds before launch for scenarios and manifest-backed `check --tests`, but runtime evidence for `Engine.time_scale`, `Engine.physics_ticks_per_second`, `Engine.max_physics_steps_per_frame`, original values, restored values, and `restoration_status` comes from Godot rather than CLI inference.

Temporal assertions (`max_delta_position`, `follows`, `never`, `within_limits`, `observe_until`) share this same runtime assertion layer in Godot for saved scenarios and live bridge commands such as `assert.observe_until`. The CLI remains the orchestration source of truth for scenario loading, deterministic sampling configuration, artifact paths, result-envelope mapping, and exit-code semantics; CLI aliases are transport ergonomics only and must not fork temporal assertion behavior.
Proximity assertions (`assert.distance`, `assert.temporal.distance_changed`) also share this same runtime target resolution and mismatch typing across scenario and live assertions. They are spatial contracts and do not replace visual assertions such as `inside_viewport`.

### 4. Artifact store

Artifacts are the memory an AI can inspect after each run.

Typical layout:

```text
.qa/artifacts/<scenario>/<run-id>/
  meta.json
  request.json
  result.json
  logs.txt
  snapshots/
    before/
      screenshot.png
      tree.json
      ui.json
      focus.json
      errors.json
    after_start/
      screenshot.png
      tree.json
      ui.json
      focus.json
      errors.json
  diffs/
    after_start.diff.png
```

### 5. Optional MCP wrapper

The MCP wrapper should be a thin layer over the CLI/library. It must not implement separate behavior.

## Execution modes

### One-shot mode

Best for CI and specs.

```bash
godot-qa scenario run sample_main_menu_start --project examples/sample_project --json
```

The CLI starts Godot, runs the scenario, captures artifacts, and exits.

### Live-session mode

Best for AI iteration.

```bash
godot-qa session start --project examples/sample_project
godot-qa input click qa:start_button --project examples/sample_project
godot-qa snapshot --project examples/sample_project --name after_start
godot-qa session stop --project examples/sample_project
```

The CLI starts Godot once and then sends commands to it.

### Hot reload (debug-host sidecar)

`session start --hot-reload` adds engine-faithful hot reload of scripts and resources. Godot's state-preserving soft reload (`GDScriptLanguage::reload_scripts(soft=true)`) and live resource reload are C++-only and reachable only over Godot's remote-debugger wire protocol, and the game always connects OUT as a debugger client — it can never listen. So the CLI must be the persistent listener. Because `session start` launches Godot detached with no supervisor, `--hot-reload` spawns a separate **debug-host sidecar** (the `godot-qa` binary re-invoked as a hidden `__debug-host` subcommand, implemented in `crates/godot-qa-debug`). The sidecar:

- binds a debugger listener and a control listener on localhost, records itself in `.qa/debug_host.json`, and Godot is launched with `--remote-debug tcp://127.0.0.1:<port>` so it dials in;
- holds the debugger connection for the session lifetime, sends `set_ignore_error_breaks`/`set_skip_breakpoints` on connect (and auto-continues any break) so a script error never blocks the game, and drains the game's debugger output stream;
- exposes a newline-delimited-JSON control channel that the transient `session reload`/`session watch` processes call; requests are translated into Godot debugger frames (`reload_scripts`, `scene:reload_cached_files`) by a minimal Godot-Variant codec in `crates/godot-qa-debug/src/variant.rs`.

Script reload (`reload_scripts`) is handled in `remote_debugger.cpp` and works in any build; resource reload (`scene:reload_cached_files`) is registered only under `DEBUG_ENABLED`, so `template_release` exports connect but silently drop it. Both frames are fire-and-forget (no acknowledgement), so resource reload is reported as best-effort/unconfirmed (`status: "requested"` plus a caveat) rather than claiming it was applied.

`session stop` reaps the sidecar alongside the game. This preserves the ownership boundary: the CLI only carries reload intent over the wire; Godot still owns the actual reload and all runtime truth.

Faithful `.tscn` **structural** live-editing (`session reload --scene`, and `.tscn` changes under `session watch`) is handled by the `godot-qa-scene` crate plus the sidecar. `godot-qa-scene` is a pure parser + differ (ported from a reference TypeScript `.tscn` parser): it parses a scene into a `SceneState` and diffs two versions into ordered `SceneEditOp`s. The sidecar owns a per-scene baseline snapshot (captured at bind from the session's launch scene), and on a `reload_structure` request it re-reads the file, diffs against the snapshot, and encodes the delta into `scene:live_*` frames (`live_set_root` once, then create/instantiate/remove/reparent and `live_node_prop` with NodePath interning) via the extended Variant codec. Because the diff works in scene-relative NodePaths keyed by `scene_file_path`, the engine re-applies each edit to every live instance. Edits that can't be expressed live (in-place node type change, root identity change, sub-resource-valued or otherwise unsupported property, no baseline) are reported as `fell_back` and the caller full-scene-reloads instead. Unlike the editor — which tracks node identity through UndoRedo — a file diff cannot always distinguish a rename from a remove+add, so a renamed/retyped node that can't be matched loses its runtime state; a matched reparent/rename preserves it.

### Live Inspection Gates

`live-gate` is a CLI-owned validation profile layered on normal live sessions. The CLI resolves manifests from `qa/live_gates/`, starts or attaches to the authenticated localhost session, orchestrates `session.status`, anchor inspection, state/probe assertions, runtime error reads, and artifact writing, then exits with the standard CLI result envelope. The Godot addon remains the source of truth for runtime scene state, locator resolution, UI geometry, focus, overlay-hidden evidence, QA method returns, probes, screenshots, and bridge errors. The gate does not add arbitrary scripting or CLI-side inference of live runtime truth.

`examples/sample_project` is the canonical checked-in consumer of the addon and CLI workflows. `cargo xtask prepare-sample` copies the canonical addon into its ignored `addons/godot_qa` directory on all supported development platforms. Repeat preparation after editing addon source.

Current contract summary:

- The exact shipped CLI surface lives in [`docs/cli-spec.md`](cli-spec.md).
- The public command inventory is intentionally defined there once, not repeated exhaustively in architecture docs.
- Recorder review remains manual: `scenario record` produces candidate artifacts plus `review.suggested_assertions`, and reviewers copy accepted assertions into `assert` before rerunning `scenario run`.

## Transport choices

Implemented transports:

- One-shot mode: CLI passes scenario/artifact paths to Godot process; Godot writes result JSON.
- Live mode: JSON lines over localhost TCP.

The live transport uses one request per TCP connection:

- CLI opens `127.0.0.1:<port>`
- CLI sends one authenticated JSON line
- Godot executes the command and returns one JSON line
- Godot closes the socket while preserving scene/runtime state in the long-lived process

For S16, pause state remains owned by Godot. `session.status` reports runtime `paused`, `session.pause` and `session.resume` toggle `get_tree().paused`, and `session.step` temporarily unpauses for exactly N frames before restoring paused state. Live time scaling follows the same ownership boundary: the CLI validates `session time-scale` arguments and sends `session.set_time_scale`, while Godot applies and reports effective `Engine.time_scale`, `Engine.physics_ticks_per_second`, and `Engine.max_physics_steps_per_frame` for the running session.

Rationale:

- Easy to debug with logs.
- Easy for a CLI to drive.
- Shared by the CLI and optional MCP server.
- No editor dependency.
- Works in CI.

## Important boundaries

- CLI can manage processes and files but cannot know runtime scene state.
- Godot bridge can know runtime scene state but should not manage global repo state.
- Scenario files are contracts; artifacts are evidence.
- Screenshot baselines are reviewed assets, not automatic truth.
- Rust owns process management, artifact filesystem concerns, CLI parsing/dispatch, transport client logic, and scenario/validation orchestration.
- Godot/GDScript remains the source of truth for live scene state, UI geometry/text/focus, screenshot capture, runtime assertions, and live edit execution.
- Visual assertion runtime evidence ownership stays in Godot: target rect/projected rect resolution, screenshot capture paths, measured visible-pixel counts/ratios, and occluder/order evidence all come from runtime state instead of CLI-side inference.
- Static `.tscn` parsing never replaces runtime inspection; preserve the `127.0.0.1` + session-token boundary.

### Android live target

Android live sessions reuse the authenticated runtime bridge instead of creating
a second execution model:

```text
CLI -> Godot debug APK export -> ADB install/private bootstrap
    -> exact localhost ADB forward -> shared live-session host
    -> private device artifacts -> binary-safe ADB pull -> host artifact store
```

The process layer owns preset parsing and ADB commands. The runtime layer owns
target-aware metadata, lifecycle, transport translation, and artifact
materialization. The addon owns runtime truth, bootstrap validation, scene
lifecycle, the serialized request loop, and private artifact paths.

Desktop `session.json` remains readable through serde-defaulted target fields.
Liveness is target-aware: desktop uses the host PID, Android queries the recorded
package on the recorded device, and an unreachable device is `unavailable`
rather than incorrectly `stopped`. Android metadata records enough information
to retry exact cleanup without discovering or removing unrelated ADB forwards or
device directories.

Android one-shot `scenario run` reuses the same deploy stages to bring up an
ephemeral scenario-mode session, then drives the scenario through the
`scenario.run` bridge command exactly like the desktop warm path. A run-scoped
artifact mapper rewrites the request's output paths beneath the session's private
device root so the engine writes result.json and evidence into app-private
storage; the CLI pulls them back into the host run tree and finalizes through the
shared verdict path, so a device run produces the same result envelope a local
process does. Device pacing is real, so the frame budget is dropped and only a
wall-clock timeout applies.
