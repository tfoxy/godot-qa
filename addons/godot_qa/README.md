# Godot QA Addon

This folder contains the Godot-side bootstrap layer for `godot-qa`.

## Install into a project

1. Copy `addons/godot_qa` into the target project's `addons/` directory.
2. Enable the `Godot QA` plugin in Godot. This registers the `GodotQaBridge` autoload singleton.
3. Activate QA mode only for test runs:
   - pass `--qa`
   - or pass one or more `--qa-*` flags such as `--qa-session-token=abc123`
   - or set the project setting `godot_qa/enabled=true` in a test-only project configuration

## Current bootstrap behavior

- The plugin installs an autoload singleton, but the runtime bridge stays dormant until QA mode is explicitly enabled.
- `qa_args.gd` parses `--qa` and `--qa-*` user args into a JSON-compatible dictionary.
- `qa_protocol.gd` emits stable success/error envelopes with `protocol_version` and optional request ids.
- `qa_editor_diagnostics.gd` is used by the editor plugin to snapshot the built-in Debugger Errors panel after editor Run starts. It writes `.qa/artifacts/editor-diagnostics/latest.json` plus timestamped `diagnostics.json` snapshots so `godot-qa diagnostics editor --json` can inspect editor-only warnings without screenshots.
- `qa_editor_profiler.gd` and `qa_editor_profiler_debugger.gd` add an experimental Godot QA Profiler editor panel. The panel starts/stops Godot's editor `servers` profiler through `EditorDebuggerSession.toggle_profiler`, captures raw debugger `servers:function_signature`, `servers:profile_frame`, and `servers:profile_total` messages, and writes `.qa/artifacts/editor-profiler/latest.json` for `godot-qa profile editor export --json`.
- `qa_snapshot.gd` provides reusable JSON-compatible helpers for node trees, all `Control` nodes plus their effective visibility flags, focus graphs, runtime errors, and screenshot files.
- `qa_runtime.gd` is the stable facade path for target resolution plus the implemented runtime commands: `tree.dump`, `ui.dump`, `focus.get`, `focus.graph`, `node.inspect`, `errors.read`, `input.tap_action`, `input.hold_action`, `input.release_action`, `input.click_target`, `input.mouse_click`, `input.key`, `input.text`, `input.joy_button`, `input.joy_motion`, saved-scenario setup commands (`setup.teleport_node`, `fixture.teleport_to_target`, `setup.spawn_scene`, `setup.set_property`, `setup.call_method`, `fixture.call`), and the `assert.*` commands. Its implementation lives behind the facade so the public preload path stays stable while large command families can be split safely. It owns the efficient `qa:` resolver, the `register_qa_id` helper for registering `qa_id` metadata with canonical `qa.id.<id>` groups, and provider registry helpers for state providers, probe providers, and fixture drivers.
- `qa_profiler.gd` is an opt-in script-span profiler for `session profile`, `scenario profile`, and `check --tests --profile`. Project scripts can preload it and wrap hot paths with `begin`/`end` or `time`; profile commands report those spans under `top_script_methods`.
- `qa_bridge.gd` dispatches live commands when QA mode is active, exposes `is_runtime_qa_active()` for projects that gate expensive pull-only QA state, requires the configured `--qa-session-token` on each live command, reports runtime `paused` state, implements `session.pause`, `session.resume`, paused `session.step`, persistent live `session.set_time_scale`, and bounded `screenshot.capture`, exposes structured runtime-error recording helpers for fixtures/runtime helpers, and routes live `assert.*` failures with stable mismatch payloads.
- `assert.distance` accepts optional `coordinate_space` or `space` values of `auto`, `world_2d`, `viewport_2d`, or `world_3d`. `auto` preserves same-space comparisons and can project 2D canvas targets into viewport space when one side is a `Control`.
- `assert.state_matches` checks generic runtime state without relying on rendered text. It can read node metadata (`source: "metadata"`), node inspect payloads (`source: "inspect"`), a registered provider (`source: "provider"`, `provider: "race_feel"`), or an explicit zero-argument QA method whose name matches `qa_*`, `get_qa_*`, or `get_*_qa_state` (`source: "method"`, `method: "get_raft_motion_qa_state"`). Registered state providers are addressed by provider ID, not node locator, and their methods do not need `qa_` naming because registration is the allowlist. Checks use `path` plus `operator: "equals" | "contains" | "present" | "absent" | "approx_equals" | "approximately_equals" | "greater_than" | "less_than"`, or a `matches` array of those checks. `approximately_equals` is an alias for `approx_equals`, and both accept optional `epsilon` with default `0.0001`. Exact numeric `equals` mismatches include full-precision numeric strings and a numeric delta when both sides are numbers.
- Visual assertion commands (`assert.visible_pixels`, `assert.not_occluded`, `assert.renders_above`, `assert.renders_below`) are screenshot-backed deterministic 2D/UI checks with runtime-owned evidence fields (screenshot path, target rect/projected rect, measured visible pixels, and thresholds). Render-order assertions accept `evidence: "auto" | "canvas_order" | "pixel_overlap"` and `require_overlap`; `auto` can use canvas layer/z/sibling evidence even when targets do not overlap. Diagnostic masks/overlays stay under `.qa/artifacts` as review candidates only and never auto-approve baselines.
- `qa_runner.gd` handles the one-shot scenario path used by `scenario run`: it loads `request.json`, instantiates the target scene, routes supported steps and assertions through `qa_runtime.gd`, captures real snapshot artifacts, and writes `result.json` with `snapshot_artifacts`.
- `qa_session_runner.gd` handles the live-session path used by `session start`: it loads one scene, presents the initial resized viewport frame to the window, binds `127.0.0.1:<random-port>`, serves one JSON-line request per TCP connection, and keeps the process alive until `session.stop`. Live sessions are realtime by default; CLI `session start --fastest` and live-script `session_start.frame_pacing` opt into launch-time `--fixed-fps`/`--disable-vsync` pacing without using `Engine.time_scale`.
- Screenshot-producing CLI flows use windowed Godot sessions by default. In forced `--headless` snapshot runs where Godot does not expose viewport pixels, generic screenshot capture may stage a visible diagnostic PNG artifact so the snapshot contract stays stable without accepting blank transparent images. Visual assertions opt out of this fallback and fail unless real viewport pixels are available.

## Assertion examples

```json
{ "distance_between": { "a": "qa:actor", "b": "qa:hud_marker", "max_distance": 80, "coordinate_space": "viewport_2d" } }
{ "state_matches": { "target": "qa:actor", "source": "metadata", "path": "qa_state", "operator": "present" } }
{ "state_matches": { "source": "provider", "provider": "race_feel", "matches": [{ "path": "phase", "operator": "equals", "expected": "gameplay" }] } }
{ "state_matches": { "target": "qa:system", "source": "method", "method": "get_qa_state", "matches": [{ "path": "ready", "operator": "equals", "expected": true }] } }
{ "state_matches": { "target": "qa:main_menu", "source": "method", "method": "get_raft_motion_qa_state", "matches": [{ "path": "mode", "operator": "equals", "expected": "steady" }] } }
{ "renders_above": { "a": "qa:marker", "b": "qa:background", "evidence": "canvas_order" } }
```

## Opt-in script profiling

Project code can opt into method timing by preloading the profiler helper and wrapping selected methods:

```gdscript
const QaProfiler = preload("res://addons/godot_qa/qa_profiler.gd")

func _process(_delta: float) -> void:
	QaProfiler.begin("PlayerController._process")
	_update_player()
	QaProfiler.end("PlayerController._process")

func rebuild_inventory() -> void:
	QaProfiler.time("Inventory.rebuild_inventory", Callable(self, "_rebuild_inventory_impl"))
```

`session profile`, `scenario profile`, and `check --tests --profile` reset these counters at the start of profiling and return ranked spans by total elapsed time at `data.profile.top_script_methods` and in `profile.json`. Uninstrumented projects return an available-but-empty `top_script_methods.methods` list.

## Experimental editor profiler export

For the Godot Editor's built-in Script Functions profiler data, enable the `Godot QA` plugin, run the game from the editor, open the `Godot QA Profiler` bottom panel, then use `Start`, `Stop & Export`, or `Export Latest`. The plugin writes:

```text
.qa/artifacts/editor-profiler/latest.json
.qa/artifacts/editor-profiler/<run-id>/profile.json
```

The CLI reads the latest editor artifact without launching or driving the editor:

```bash
godot-qa profile editor export --project <project> --json
```

This source is marked `experimental` because it depends on Godot editor debugger message names and the source-derived raw frame layout, not a stable high-level API for reading the visible Profiler table. It does not replace the opt-in runtime `top_script_methods` profiler.

## Current scope limits

- project validation/import/check orchestration still lives on the CLI side rather than inside the addon runtime contract
- Live layout audits and recorder layout checks use the same runtime geometry and focus evidence; they do not judge visual design quality.

## Safety expectations

- Do not ship the QA addon in release exports by default.
- Do not rely on the autoload existing as proof that QA mode is active.
- Expect inactive commands to fail with a structured `bridge_inactive` error until QA mode is enabled.
- Expect live commands to require both localhost reachability and the per-session token; the token value belongs only in `.qa/session.token`, not normal CLI output.
- Editor diagnostics capture is best-effort because Godot does not expose a stable public data API for every built-in Debugger Errors row. The helper isolates editor UI traversal and leaves `diagnostics editor` able to fall back to process/log capture.
- Editor profiler export is best-effort and version-aware. If raw debugger profile data is unavailable, the artifact keeps warnings such as `editor_profiler.debug_data_unavailable` instead of inventing script-function rows.

## Android live bootstrap

On Android debug template builds, the autoload may consume the fixed private
`user://godot_qa/android_live_bootstrap.json` file written by the CLI. It
validates schema version, session ID, token, port, optional `res://` scene, and
project arguments; derives `user://godot_qa/live/<session-id>`; initializes the
requested scene and arguments; starts the same loopback-only serialized host as
desktop; then deletes the one-shot file.

This path is inactive for release builds, desktop builds, editor runs, malformed
bootstrap, or missing explicit bootstrap. Add the autoload and addon files to an
Android debug APK and grant `INTERNET`; do not include the QA bridge in release
exports.
