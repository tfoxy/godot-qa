# Scenario Format

Scenarios describe reproducible UI/gameplay flows.

JSON scenario files are always supported. The CLI also supports a built-in YAML subset for the checked-in scenario shape; install `godot-qa[yaml]` only if you need broader YAML syntax.

Current implemented support is intentionally narrower than the full format below:

- steps: `wait_frames`, `wait_until`, `snapshot`, `tap_action`, `hold_action`, `release_action`, `input_joy_button`, `input_joy_motion`, `input_joy_state`, `drive_policy`, `click`, `node_inspect`, `record_state`, `teleport_node`, `fixture_teleport`, `spawn_scene`, `set_property`, `joy_connect`, `joy_disconnect`, `call_method`, `fixture_call`, `audio_capture`, `audio_export`, `physics_overlay`
- assertions: `exists`, `not_exists`, `visible`, `hidden`, `enabled`, `disabled`, `focused`, `text_equals`, `text_contains`, `inside_viewport`, `no_overlap`, `no_errors`, `state_matches`, `surface_probe`, `surface_matches`, `layout_audit`, `screenshot_match`, `visible_pixels`, `saved_snapshot_pixels`, `not_occluded`, `renders_above`, `renders_below`
- `snapshot` writes `screenshot.png`, `tree.json`, `ui.json`, `focus.json`, and `errors.json` under `snapshots/<snapshot-name>/`
- `visible` and `hidden` use effective `CanvasItem` scene-tree visibility, not just the node's local `visible` flag
- `visible_pixels`, `saved_snapshot_pixels`, `not_occluded`, `renders_above`, and `renders_below` are screenshot-backed visual assertions for deterministic 2D/UI targets; they do not replace `visible`, `hidden`, layout checks, or manually reviewed screenshot baselines
- visual assertion evidence requires snapshot-backed geometry and pixel measurements: screenshot path, `target_rect` or `projected_rect`, measured `visible_pixel_count`/`visible_pixel_ratio`, configured threshold (`min_count`/`min_ratio` when present), and `occluder`/`other` for occlusion and order checks when available
- Optional `scenario run --visual-risk` analysis consumes screenshot artifacts after the run and emits machine-readable warnings for generic placeholder/blockout risk signals. It is not static scenario lint, not aesthetic approval, and not a replacement for screenshot baselines or screenshot-backed visual assertions.
- `qa:`, `path:`, and `name:` locators must resolve uniquely at runtime; ambiguous matches fail with `type: "ambiguous_target"` and list the matching node paths in `actual`
- Scenarios should target durable `qa:` anchors for important systems and visible controls, then use state/probe assertions for inspectable gameplay facts. Parse debug labels only when the label text itself is the player-facing contract.
- unsupported runtime step/assertion failures report the rejected operation name in `target`, the supported operation set in `expected`, and the rejected step/assertion object in `actual`
- malformed top-level `viewport`, `determinism`, or `snapshots` values fail validation unless they are objects; `assert` must be a list instead of being silently ignored
- `godot_args` and `project_args` are optional scenario launch argument arrays. Each field must be a list of non-empty strings, not shell syntax. `godot_args` are passed to Godot before the `--` separator; `project_args` are passed after the separator to the project/game. Both fields may appear on the same scenario and preserve order on their own side of the separator. Project-wide config defaults from `qa/config.yaml` are prepended before scenario-specific args.
- `slow_summary_threshold_ms` is optional and defaults to `1000`. It only controls informational slow-summary evidence written from completed progress heartbeat events. A scenario can also use `slow_summary: {threshold_ms: <n>}`. Slow thresholds do not fail successful runs; explicit timeouts are the failure mode for hung runs.
- `timeout_seconds`, `timeout`, `run_timeout_seconds`, and `run_timeout` are optional run timeout aliases. Step/assertion entries may use `timeout_seconds` or `timeout` for operation-local limits when supported by the runner. Numeric values are seconds; duration strings support `ms`, `s`, and `m` suffixes. Values must be positive and finite. Zero, negative numbers, non-numbers, empty strings, NaN/infinity, and unsupported duration strings fail before Godot launch with `type: "scenario_validation_failed"` rather than becoming assertion failures.
- malformed `godot_args` or `project_args` values fail before artifact creation or Godot launch with `type: "scenario_validation_failed"`, `command: "scenario run"`, an actionable message, the expected list-of-non-empty-strings shape, and the invalid field value in `actual`.
- Progress artifacts redact sensitive context. Scenario `progress.jsonl` may include active step/assertion indexes, operation kind, and safe target/locator context, but it must not expose live session tokens or unredacted secret-looking project args.
- Persisted summary metadata redacts secret-looking `project_args` while preserving argument count and order. Values following secret-bearing keys such as `--token`, `--password`, or `--client-secret` are stored as `<redacted:N>`, and secret-looking `KEY=VALUE` entries such as `API_KEY=supersecret` are stored as `API_KEY=<redacted:N>`, where `N` is the original value character count. Runtime request payloads sent to Godot still contain the raw values needed to launch the project.
- malformed payloads for supported operations fail CLI validation before artifact creation or Godot launch:
  - `wait_frames` must be a non-negative integer
  - `wait_until` must be an object with `source` (`provider` or `method`), the matching `provider` or `target`+`method`, a non-empty dot-separated `path`, a supported `operator`, `expected` for value operators, and a positive integer `timeout_frames`; optional `poll_every_frames` must be a positive integer
  - `tap_action`, `release_action`, `click`, `node_inspect`, and `exists|not_exists|visible|hidden|enabled|disabled|focused|inside_viewport` must use non-empty strings
  - `hold_action` must be an object with non-empty string `action` plus non-negative integer `frames`
  - `input_joy_button` must be either a non-empty button string or `{button, optional device, optional frames}` with non-negative integer `device`/`frames`
  - `input_joy_motion` must use `{axis, value, optional device, optional frames}` with a non-empty axis string, numeric value from `-1.0` to `1.0`, and non-negative integer `device`/`frames`
  - `input_joy_state` must use `{inputs, optional frames}` where `inputs` is a non-empty list and each entry is `{device, motion|button}` with a non-negative integer `device` and exactly one of `motion` (`{axis, value}`, non-empty axis string, numeric value from `-1.0` to `1.0`) or `button` (`{name, pressed}`, non-empty name string, boolean `pressed`); optional `frames` must be a non-negative integer. Unknown top-level fields are rejected. The whole list is validated before anything is injected
  - `record_state` must use `{name, source, ...}` where `name` is a required non-empty safe artifact segment (letters, numbers, `.`, `_`, `-`, like a `snapshot` name) that must be unique across the scenario's `record_state` steps, and `source` plus its companions reuse the `wait_until`/`state_matches` state-source vocabulary: `source: provider` needs a non-empty `provider`; `source: method` needs a non-empty `target` and `method`. Optional `path` is a non-empty dot-separated selection into the read value. Unknown fields are rejected
  - `joy_connect` must use `{devices, optional name}` where `devices` is a non-empty list of unique non-negative integers and `name` (connect only) is an optional non-empty string; unknown fields are rejected
  - `joy_disconnect` must use `{devices}` with a non-empty list of unique non-negative integers (no `name`); unknown fields are rejected
  - `drive_policy` must use `{policy, devices, max_frames}` with a non-empty policy string, a non-empty list of non-negative integer `devices`, and a positive integer `max_frames`; optional `params` must be an object, optional `tick_every_frames` must be a positive integer, and optional `until` uses the `wait_until` predicate fields without `timeout_frames` (the step's `max_frames` is the budget)
  - `teleport_node` must use exactly one of `{target, to}`, `{target, position}`, or `{target, to_point}`; `to` is a non-empty locator, `position.x/y` are numeric, and `to_point` is a read-only point-provider request
  - `fixture_teleport` must use `{target}` plus exactly one of `to`, `position`, or `to_point`; optional `offset` and `preserve_velocity` are supported
  - `spawn_scene` must use a non-empty `scene` path plus optional non-empty `parent`, `name`, `qa_id`, and either `at` or `position`
  - `set_property` must use non-empty `target` and `property` fields plus a `value` field
  - `call_method` must use non-empty `target` and a `method` beginning with `qa_`; optional `args` must be a list
  - `fixture_call` must use non-empty `driver` and `action` fields; optional `params` must be an object
  - `audio_capture` must use `{name, duration_seconds}` with optional `bus` and `analyze`; names are artifact-safe identifiers, duration must be positive and bounded to 30 seconds, `bus` must be a non-empty string when present, and `analyze` must be boolean when present
  - `audio_export` must use `{name, target, method}` with optional `args` and `analyze`; names are artifact-safe identifiers, `target` and `method` must be non-empty strings, `args` must be a list when present, and `analyze` must be boolean when present
  - `physics_overlay` must use either a non-empty `target` or `viewport: true`; optional `name`, `snapshot`, `radius`, `frames`, `include_aabb`, `include_contacts`, `state_provider`, `state_method`/`state_methods`, `no_project_state`, and `max_shapes` follow the live `physics overlay` bounds and read-only provider rules
  - `text_equals` and `text_contains` must use `{target, expected}` objects with non-empty string `target` plus string `expected`
  - `no_overlap` must use `{a, b}` with non-empty string locators
  - `screenshot_match` must use `{snapshot, baseline}` with non-empty strings, and optional `threshold` must be a number from `0.0` to `1.0`
  - `visible_pixels` and `saved_snapshot_pixels` must use `{target, snapshot}` plus at least one of `min_ratio` or `min_count`; `min_ratio` must be from `0.0` to `1.0` and `min_count` must be non-negative
  - `not_occluded` must use `{target, snapshot}` plus optional `occluder`; `renders_above` and `renders_below` must use `{target, other, snapshot}`
  - `no_errors` must be a boolean
- `screenshot_match` baselines must resolve inside `qa/baselines`; absolute paths and path traversal are rejected as `baseline_path_unsafe`
- `screenshot_match` is single-viewport only; combining it with a multi-entry `viewports` matrix is rejected statically at validation as `scenario_validation_failed` (the lint-time mirror of the runtime `screenshot_match_viewports_unsupported` guard). A single `viewports` entry is fine, and `screenshot_match` may now coexist with `layout_audit` on that single run
- `scenario run` launches Godot windowed when a scenario requests screenshot artifacts, uses `screenshot_match`, uses screenshot-backed visual assertions, or runs a rendered layout/viewport matrix. Screenshot-backed scenarios require real rendered media evidence: the CLI may use `godot.windowed_launch_prefix` or a safely detected virtual display wrapper, otherwise it fails before runtime execution with `type: "display_backend_unavailable"`. Scenarios without media evidence may still run headless, and `audio_export` remains headless unless another media requirement exists. The audio driver is an independent axis: `scenario run --audio` / `--no-audio` (and `godot.audio_mode: auto|on|off`) select the real audio driver or `--audio-driver Dummy`, mirroring how `scenario run --headless` / `--windowed` (and `godot.launch_mode`) override the automatic display choice; forced `--headless` media runs may fail later with the existing screenshot/audio capability errors. Live screenshot commands and `scenario record` require windowed sessions.
- Screenshot-backed scenarios require real rendered pixels; audio capture similarly requires live runtime media evidence rather than synthetic fallback artifacts.
- Visual assertion diagnostic screenshots, masks, and overlays are review artifacts under `.qa/artifacts`; approving or updating files under `qa/baselines` remains a separate manual action.
- Unsupported visual cases (for example ambiguous locator resolution or runtime limits outside deterministic 2D/UI evidence) fail with structured errors instead of returning synthetic confidence.
- Audio capture/export is explicit. Scenario runs do not record audio by default; a run records or exports audio only when a step requests `audio_capture` or `audio_export`.
- In automatic mode `audio_capture` selects the real audio driver (instead of the default `--audio-driver Dummy`) but does NOT force a windowed launch: it runs headless-display + real-audio unless the same scenario also requests screenshots, visual assertions, or layout/viewport rendered output. `audio_export` needs neither a window nor the real audio driver; it calls a project-owned QA hook and stays headless with the Dummy audio driver unless another media requirement exists.
- `process_outcome_policy.known_wrapper_exit_codes` is an explicit opt-in for known wrapper-only non-zero Godot process exits. The accepted shape is an object with a non-empty `known_wrapper_exit_codes` list of positive integers, for example `{"process_outcome_policy":{"known_wrapper_exit_codes":[7]}}`. The policy is considered only after `result.json` is present, structurally valid, `ok` is `true`, runtime failures are empty, and relevant logs are clean. A listed non-zero exit then returns `data.outcome.status: "pass_with_warning"`, keeps top-level `ok: true` and `failures: []`, and records the machine-readable warning under `data.process.warnings`. Missing or malformed runtime results, runtime failures, log errors or warnings, absent policy, and unlisted exit codes remain strict failures. Scenario assertion syntax is unchanged; this metadata only affects final process-outcome mapping after runtime assertions and artifact validation have already succeeded.

## Review Template Guidance

`review init` emits normal scenario files or normal live-session script files. Generated scenario review files use the same top-level shape as hand-authored scenarios: `name`, `scene`, optional `project_args`, `snapshots`, `steps`, `assert`, and a review metadata block. Generated live-script files use `name`, `steps`, and a review metadata block. The `review` block records the template ID, generated command type, run command, selected targets and timing parameters, warnings, and notes for humans; it is metadata only and is not an assertion surface.

Generated scenario files are run with:

```bash
godot-qa scenario run --file .qa/review-scenarios/<name>.yaml --project <project> --json
```

Generated live scripts are run with:

```bash
godot-qa session run-script .qa/review-scripts/<name>.yaml --project <project> --json
```

Review template generation must stay inside documented operations. Scenario templates may use supported scenario steps and assertions such as `wait_frames`, `snapshot`, and `no_errors`. Live-script templates may use supported live-script operations such as `session_start`, bounded waits, `fixture_call`, `screenshot_sequence`, `trace_record`, `physics_collision_report`, `audio_capture`, `event_trace`, `tree`, `ui`, `logs`, `errors`, `no_errors`, and `session_stop`. A template must omit optional operation groups when required caller inputs are absent, or return a structured generation failure; it must not invent project-specific YAML commands.

`project_args` are forwarded exactly as supplied. In generated scenarios they remain a top-level `project_args` list; in generated live scripts they are placed under the `session_start` step. They preserve string order, are passed after Godot's `--` separator, and are not interpreted by the CLI. Values such as `--seed`, save-slot names, or domain-specific flags remain project-owned launch arguments.

Review templates are project-agnostic. Template IDs and generated comments/metadata must not assume racing games, rafts, combat arenas, player controllers, or other project semantics. Durable `qa:` anchors, caller-supplied scenes, targets, event providers, frame counts, screenshot cadence, audio duration, and output paths are the supported boundaries.

Generated comments and metadata are there to explain how to run the file and why evidence was captured. They accelerate exploratory QA, but final human review, assertion authoring, and screenshot baseline approval remain separate manual decisions.

## Physics Overlay Steps

Use `physics_overlay` when a scenario should preserve collider-overlay evidence for later review without adding a visible debug overlay to the running game:

```yaml
steps:
  - snapshot: before_overlay
  - physics_overlay:
      name: overlay_actor
      target: qa:overlay_actor
      snapshot: before_overlay
      radius: 400
      frames: 2
      include_aabb: true
      include_contacts: false
      state_method: get_qa_collision_visual_state
```

Fields:

- `target`: target locator for focused collection. Required unless `viewport: true` is set.
- `viewport`: optional boolean for viewport-wide collection. It is mutually exclusive with a target in CLI review usage.
- `name`: optional artifact-safe evidence key suffix. When omitted, the runner keys evidence by step index plus target or `viewport`.
- `snapshot`: optional saved snapshot name to associate with the metadata. When omitted, the runner uses the most recent prior snapshot step if one exists.
- `radius`: optional positive finite radius up to `4096`; defaults to the live collector default.
- `frames`: optional frame delay from `1` through `240`.
- `include_aabb`: optional boolean for AABB evidence.
- `include_contacts`: optional boolean for contact point/normal evidence when live physics data exists.
- `state_provider`, `state_method`/`state_methods`, `no_project_state`, and `max_shapes`: optional provider and bounding controls matching `godot-qa physics overlay`.

Scenario runs store the raw collector payload in `result.json.physics_overlays`, keyed by step index/name or target. The entry records `step_index`, `name`, `target`, `viewport`, optional `snapshot`, and `metadata`. It does not write overlay PNGs during the scenario run. To render saved evidence later, run:

```bash
godot-qa physics overlay qa:overlay_actor --project examples/sample_project --scenario physics_overlay_demo --snapshot aligned --out .qa/artifacts/manual/physics_overlay_from_snapshot.png --json
```

That review command reads the saved `snapshots/<snapshot>/screenshot.png` and matching `result.json.physics_overlays` metadata, then writes `<stem>.base.png`, `<stem>.png`, and `<stem>.overlay.json` beside the requested `--out` path. Missing saved screenshots or matching overlay metadata are artifact failures and do not trigger a fresh runtime capture.

Warnings in overlay evidence use the `physics_overlay.*` namespace and remain advisory unless reviewed with CLI `--strict`. Visual-footprint comparison warnings require an approved read-only project provider such as `get_qa_visual_footprint_state()` or `get_qa_collision_visual_state()`. Without provider data, scenarios must not infer visual footprint size from sprites, textures, object names, or debug labels.

Physics overlay steps produce review artifacts and saved evidence only. They do not approve screenshot baselines, do not update `qa/baselines`, do not mutate collision shapes, and do not install a game-facing debug HUD.

## Event Trace Guidance

Use `godot-qa event trace` beside scenarios when a fixture needs to validate that gameplay, VFX, SFX, UI, collision, or custom QA events happened in the same bounded window without visible debug labels. The event trace is the supported generic event stream path; `trace record --events-provider` is not implemented in the current public command surface, so existing `trace record` scenarios keep their current behavior.

Projects expose events through an approved read-only provider method on a durable target, using one of `get_qa_event_stream_state`, `get_event_stream_qa_state`, `get_qa_events`, or `qa_event_stream`. Event type strings are project-provided strings. Scenario guidance and pairing configs must not make game-specific event names part of the generic contract.

Event traces are for machine-readable correlation, not player-facing UI. Do not add visible debug labels, hidden debug-label parsing, event overlays, or screenshot inference to prove that audio, VFX, UI, or collision events fired. If a fixture needs SFX/VFX correlation, expose explicit provider events and validate the resulting `events.json`, `summary.json`, warning IDs, and optional pairing/rate-limit config.

Example validation commands for a deterministic fixture:

```bash
cargo run --bin godot-qa -- event trace --project examples/sample_project --target qa:event_source --provider get_qa_event_stream_state --frames 120 --sample-every-frames 1 --json
cargo run --bin godot-qa -- event summarize .qa/artifacts/live/latest/events/latest/events.json --json
```

## Audio Steps

Use `audio_capture` when the runtime can capture a Godot bus for a short deterministic window:

```yaml
steps:
  - audio_capture:
      name: dodgeball_music
      duration_seconds: 4.0
      bus: Master
      analyze: true
```

Fields:

- `name`: required artifact-safe identifier using only letters, numbers, `.`, `_`, and `-`. The runner writes `audio/<name>.wav` and, when analysis runs, `audio/<name>.analysis.json`.
- `duration_seconds`: required positive finite number up to `30.0`.
- `bus`: optional non-empty Godot audio bus name. Defaults to `Master`.
- `analyze`: optional boolean. Defaults to enabled by the runtime/CLI path when an audio artifact is produced.

Use `audio_export` when a project owns deterministic audio generation and can expose a QA hook:

```yaml
steps:
  - audio_export:
      name: generated_dodgeball_loop
      target: qa:audio_controller
      method: qa_export_audio_track
      args: ["dodgeball_rush_loop"]
      analyze: true
```

`audio_export.method` must resolve at runtime to a method whose name begins with `qa_`. The runtime rejects non-`qa_` methods with `type: "audio_export_rejected"` and includes the command, target, rejected method, expected pattern, and actual method. The QA method receives the scenario `args` followed by a context dictionary containing the controlled `audio_dir`, normalized `name`, intended `wav_path`, and `max_bytes`. It may return a `PackedByteArray` containing RIFF/WAVE bytes, a base64 string containing RIFF/WAVE bytes, or a JSON-compatible dictionary with `wav` or `path` pointing to an existing WAV file inside the controlled audio artifact directory.

Audio export payloads are bounded to `10,485,760` bytes at the runtime boundary. Oversized or empty payloads fail with `audio_export_oversized`; non-RIFF/WAVE bytes fail with `audio_export_invalid_return`; non-JSON-compatible dictionaries fail with `audio_export_invalid_metadata`; and paths outside the controlled audio directory fail with `audio_export_path_rejected`. `audio_capture` reports unavailable recording APIs or missing buses as `audio_capture_unavailable` and write/recording errors as `audio_capture_failed`.

Audio analysis reports measurable facts only: duration, sample rate, channel count, peak/RMS level, silence or near-silence evidence, clipping sample count, loop-boundary delta, transient count, zero-crossing rate, and coarse spectral buckets. It does not judge whether music sounds good, approve loops, or replace manual listening review.

## Screenshot-backed visual assertions

Use `visible_pixels` when the player-visible contract is that a target contributes enough real rendered pixels in a freshly captured runtime snapshot. Use `saved_snapshot_pixels` when a saved snapshot artifact already contains the real screenshot and viewport-space target evidence needed to measure useful object pixels without recapturing the whole scenario. Use `not_occluded` when the target should remain measurably visible after possible blockers are drawn. Use `renders_above` and `renders_below` when the contract is relative draw order between two deterministic 2D/UI targets. These assertions are separate from `visible` and `hidden`, which inspect effective scene-tree visibility state, and from screenshot baselines, which remain manually reviewed full-frame assets.

Screenshot-backed visual assertions require viewport-space evidence. Built-in geometry covers supported `Control`, `CanvasItem`, and `Line2D` targets when the runtime can derive a rect or projected rect. Dynamic gameplay UI, debug overlays, custom drawing, or non-standard nodes should expose `qa_visual_rect()` or `qa_visual_rect` metadata returning `{x, y, width, height}`, `[x, y, width, height]`, or `Rect2`. Non-`CanvasItem` targets that use this path must also expose `qa_set_visual_visible(visible)` so pixel-delta evidence can hide the target safely. Use QA-prefixed visual evidence methods for rendered contracts only; use non-visual state/probe assertions such as QA-prefixed state readers or surface probes when the contract is gameplay state, AI state, physics classification, or other inspectable logic rather than player-visible pixels.

Headless visual-pixel assertions require real rendered pixels and fail when Godot cannot expose screenshot evidence. They must not pass by reading scene-tree visibility, debug labels, diagnostic fallback images, or synthetic screenshots. Unsupported visual targets fail with stable runtime error types such as `unsupported_visual_assertion`, `unsupported_coordinate_space`, or `screenshot_capture_failed` and include remediation guidance for adding `qa_visual_rect`, `qa_set_visual_visible`, a supported coordinate-space hook, or a real display backend.

Target geometry is reported in the 2D canvas coordinate space (`get_global_rect()`/`qa_visual_rect`), but screenshots are captured at the OS window resolution. When content scaling makes the window resolution differ from the canvas (for example a scenario with `godot_args: ["--resolution", "960x540"]` while the project canvas is `1920x1080` under `display/window/stretch/mode="canvas_items"`), the runtime converts the canvas rect into screenshot space (via `Window.get_final_transform()`) before counting pixels, so the reported `visible_pixel_ratio`/`visible_pixel_count` stay correct. `visible_pixels` evidence therefore includes both `target_rect` (canvas space, unchanged) and the additive `target_rect_screenshot` (the transformed rect actually sampled), plus `canvas_to_screenshot` (`{scale: [sx, sy], offset: [ox, oy]}`) and `screenshot_size`. When the window matches the canvas the transform is the identity and `target_rect_screenshot` equals `target_rect`. If the transformed full-canvas rect does not match the captured screenshot size (an unforeseen capture/scaling mismatch), the assertion fails loudly with `screenshot_geometry_mismatch` instead of sampling the wrong pixels. `saved_snapshot_pixels` reuses saved snapshot `visual_evidence`, which now records `rect_screenshot` alongside `rect`; consumers prefer `rect_screenshot` when present and fall back to `rect` so older snapshots keep working.

Visual-risk analysis is a separate optional `scenario run` post-processing surface. With `--visual-risk`, the CLI reads the screenshot artifacts and visual evidence already produced by the completed run, then reports warning-level findings under `data.visual_risk`. With `--visual-risk-strict`, the same findings become failures. These findings use the `visual-risk.*` namespace and only flag generic risk signals such as low visual diversity, huge flat shapes, excessive debug-primary colors, or targets with missing visible pixels. They do not decide whether the art is good, approve or reject baselines, compare against baselines, or weaken the existing `screenshot_low_information` failure behavior.

`saved_snapshot_pixels` fields:

- `target`: locator or QA ID to resolve inside the saved snapshot context.
- `snapshot`: saved snapshot name whose `screenshot.png` and JSON context should be measured.
- `min_count`: optional non-negative useful-pixel count threshold.
- `min_ratio`: optional useful-pixel ratio threshold from `0.0` through `1.0`.

At least one threshold is required, and invalid thresholds fail scenario validation before the assertion can pass. The saved snapshot must include a real, non-low-information screenshot and a matching target with `visual_evidence.coordinate_space: viewport_2d` plus a positive rect. A target that is scene-tree visible but contributes no useful pixels fails with `visible_pixels_threshold_mismatch`; blank, transparent, all-black, visually flat, stale, missing, or baseline-path screenshots fail with stable artifact or screenshot-quality failures instead of passing. Use live `visible_pixels`, `not_occluded`, `renders_above`, and `renders_below` when target-hidden captures or current runtime draw-order evidence are part of the contract.

## Lint metadata (S37)

`godot-qa scenario lint` reads saved scenario files statically and reports manual-play integrity risks plus contract errors.

Rule IDs and intent:

- `GQA-LINT-001` (warning): detects fixture teleports/direct state mutation near assertions that should be exercised through player-facing input. Why it matters: fixture-heavy flows can hide interaction regressions. Suggested fix: keep deterministic setup at the start, then drive behavior with real inputs before asserting.
- `GQA-LINT-002` (warning): detects assertions that only check debug labels/tokens for an interactive flow. Why it matters: debug text can pass while player-visible UI is wrong, and a hidden debug label should not be the primary automation API when a method/probe assertion can inspect the same state. Treat this as a risk signal for debug/status-token-only assertions. Suggested fix: pair debug checks with visible/focus/layout/visual assertions on player-facing targets, or expose QA-prefixed state/probe methods for non-visual gameplay facts.
- `GQA-LINT-003` (warning): detects scenarios that claim visual coverage but do not opt into screenshot capture/snapshot evidence. Why it matters: no rendered evidence means visual regressions can be missed. Suggested fix: enable snapshot screenshot capture and add screenshot-backed or baseline-reviewed checks.
- `GQA-LINT-004` (warning): detects interactive scenarios that never assert an unwanted state is absent. Why it matters: positive-only assertions can miss regressions that appear alongside expected state. Suggested fix: add at least one negative assertion (`hidden`, `not_exists`, or equivalent) for the risky state.
- `GQA-LINT-005` (warning): detects screenshot-backed visual coverage without pixel, occlusion, or render-order guards. Why it matters: an obstructed screenshot can look captured while missing the player-visible target. Suggested fix: add `visible_pixels`, `not_occluded`, `renders_above`, or `renders_below` for the important target.
- `GQA-LINT-006` (warning): detects high-cardinality `qa:` locators such as per-cell, per-station, per-island-area, or per-pixel helper IDs. Why it matters: generated IDs add runtime metadata churn and make scenarios depend on implementation detail. Suggested fix: use durable anchors plus `to_point`, QA probes, or aggregate `state_matches` selectors.
- `GQA-LINT-007` (warning): detects `state_matches` assertions that read node metadata as dynamic state. Why it matters: metadata mirroring is expensive and brittle when a QA method can return the same facts directly. Suggested fix: use `source: method` with a QA-prefixed state method.
- `GQA-LINT-008` (warning): detects scenarios that explicitly claim project flag coverage with `lint.validates_project_flag: true` or `lint.validates_project_args: true` but do not provide non-empty `project_args` or documented equivalent fixture metadata. Why it matters: project/game flags are passed after Godot's `--` separator, so a scenario claiming flag coverage should show how that flag reaches the project. Suggested fix: add `project_args` or document the replacement fixture with `lint.project_args_fixture` or `lint.equivalent_project_args_fixture`.
- `scenario_document_normalized` (warning): detects scenario files whose raw shape differs from the normalized form used by the runner. Why it matters: dropped or normalized fields can hide drift between what authors wrote and what the CLI executes. Suggested fix: align the file with the normalized scenario shape.
- `missing_qa_id_reference` (warning): detects `qa:` locators that are absent from static project sources and not created by a prior `spawn_scene` step in the same scenario. Why it matters: likely-broken locators reduce assertion value before runtime. Suggested fix: correct the locator or add the missing `qa_id` metadata.
- Scenario templating errors surface at load time, before lint rules run, so a broken `extends`/`vars` block fails loudly rather than as a soft finding. A missing `extends` base is a hard load failure (`scenario_extends_missing`), not a name-lookup lint finding. `scenario lint` additionally validates abstract templates structurally and reports malformed `extends`/`vars` blocks as `invalid_scenario_file`. See "Scenario templates (extends + vars)" for the full error-kind table.
- `duplicate_scenario_name` (error): detects duplicate resolved scenario names in the same `qa/scenarios` tree. Why it matters: ambiguous resolution makes automation unreliable. Suggested fix: rename scenarios so each resolved name is unique.
- `malformed_suppression` (error): detects malformed or unsupported suppression metadata. Why it matters: malformed suppression should not silently hide findings. Suggested fix: use the documented suppression shape with a non-empty reason.
- Contract preflight errors also use stable rule IDs such as `invalid_scenario_file`, `project_missing`, `scenarios_dir_missing`, `scenario_not_found`, and `scenario_ambiguous`.

Suppressions are YAML/JSON-friendly and require a reason:

```yaml
lint:
  suppressions:
    GQA-LINT-002:
      reason: "debug HUD text is the user-facing contract for this scenario"
```

Suppression rules:

- Use `lint.suppressions.<rule_id>.reason` with a non-empty string.
- Suppressions apply only to suppressible warning rules.
- Contract errors are never suppressible and remain failing findings.
- Suppressed warnings are omitted from `data.findings` and retained under `data.suppressed_findings` with suppression metadata.
- Suppress `GQA-LINT-002` only when the label text is genuinely player-facing, such as a HUD status label whose words are the behavior being validated.

## Example

```yaml
name: sample_main_menu_start
scene: res://ui/main_menu.tscn

viewport:
  width: 1920
  height: 1080

determinism:
  fixed_fps: 60
  seed: 123
  locale: en
  test_mode: true

snapshots:
  save_tree: true
  save_ui: true
  save_screenshot: true
  save_logs: true
  fail_on_low_information: true
  target: qa:start_button
  max_depth: 1
  qa_only: false
  include_metadata: true
  metadata_keys: [qa_id, role]

steps:
  - wait_frames: 5
  - snapshot: before
  - click: qa:start_button
  - wait_frames: 10
  - snapshot: after_start

assert:
  - exists: qa:pause_menu
  - visible: qa:pause_menu
  - focused: qa:resume_button
  - hidden: qa:main_menu_buttons
  - no_errors: true
  - screenshot_match:
      snapshot: after_start
      baseline: qa/baselines/sample_main_menu_start/after_start.png
      threshold: 0.01
```

## Required fields

```text
name
scene
steps
```

`name` must also be safe for artifact paths because `scenario run` writes runs under `.qa/artifacts/<name>/<run-id>/`. Use only letters, numbers, `.`, `_`, and `-`.
Within a given `qa/scenarios` tree, `name` should also be unique. `scenario run <name>` now fails with `type: "scenario_name_ambiguous"` if multiple scenario files declare the same resolved name.
`scenario record <name> --scene ...` writes candidate scenarios under `qa/scenarios/candidates/<name>.json` by default and refuses to overwrite an existing file unless an explicit future overwrite flow is added.

When `snapshots.save_screenshot` is true, `scenario run` records screenshot quality metadata in `data.screenshot_quality` for each saved screenshot it can read. Low-information screenshots, including transparent images, blank images, tiny images, or visually flat captures, fail the scenario with `type: "screenshot_low_information"` by default. Screenshot-backed scenarios require a real host display or safely detected virtual display wrapper; if neither is available, they fail as `display_backend_unavailable` instead of producing synthetic evidence. `snapshots.fail_on_low_information` remains accepted for compatibility, but low-information screenshots are always treated as failures.

Visual-risk analysis is intentionally not part of `scenario lint`. `scenario lint` remains static and keeps its existing warning/error exit semantics; it does not load `.qa/artifacts`, inspect screenshots, or emit `visual-risk.*` findings unless a future spec explicitly adds artifact-consuming lint behavior.

Snapshot filters are optional top-level `snapshots` fields. With no `target`, `max_depth`, `qa_only`, `include_metadata`, or `metadata_keys`, scenario snapshots keep the compatible full-artifact behavior. Use filters when a scenario needs bounded evidence from a large project, for example a focused subtree around a stable `qa:` anchor. Use full snapshots for broad debugging, layout audits, artifact review, and assertions that may need nodes outside a scoped subtree.

- `target`: non-empty locator that scopes JSON capture to the resolved subtree.
- `max_depth`: non-negative integer depth limit below the capture root.
- `qa_only`: boolean that keeps QA-bearing nodes and the ancestors/descendants needed to read them.
- `include_metadata`: boolean; omit it for current compatible metadata behavior, set it to `false` to omit metadata from JSON artifacts.
- `metadata_keys`: list of safe metadata keys to serialize when metadata is included. Keys must be non-empty and use only ASCII letters, numbers, `.`, `_`, `-`, and `:`.

Malformed filter config fails scenario validation before runtime where possible. Runtime target failures use the normal target failure contract. Filtering affects `tree.json`, `ui.json`, and `focus.json` style evidence; screenshots are not cropped or filtered, and `errors.json` remains available.

## Recommended fields

```text
viewport
determinism
snapshots
assert
godot_args
project_args
```

## Scenario templates (extends + vars)

Families of near-identical scenarios (same scene, same flow, different tuning) create copy-paste drift: change one shared step and you must edit every file. Scenario templating lets one abstract base own the shared document while concrete children re-tune it through variables. Resolution happens once, at load time, so every consumer (`scenario run`, `scenario warm`, bundles, `scenario lint`, session scripts) still sees a plain, fully-resolved document. Files that use none of the template keys (`extends`, `vars`, `abstract`) take a fast path and are unchanged.

### `extends`

`extends` names a base scenario file as a path **relative to the current file's directory**. Absolute paths are rejected, and the path must end in `.yaml`, `.yml`, or `.json` (a YAML child may extend a JSON base and vice versa). Chains are allowed (a base may itself `extends` another base) up to a depth of 8; a cyclic chain (`scenario_extends_cycle`) or an over-deep chain (`scenario_extends_invalid`) is rejected. A base that is missing or unreadable fails with `scenario_extends_missing`; a base that parses badly propagates the base's own parse-failure kind with the extends chain in its message.

`name` and `abstract` are **never inherited** from a base: only the concrete child names the scenario, and a child of an abstract base is not itself abstract.

### Merge rules

The base body (minus its `name`/`abstract`) is deep-merged with the child, child-winning:

- **Objects** merge key-by-key (so a child can override just `viewport.height`).
- **Arrays and scalars replace wholesale.** There is no array-element merge: to change one step you must either restate the whole `steps` list, or parameterize the step with a var. Replacing `steps` replaces all of it.
- A post-merge key whose value is literal `null` is **deleted** at any depth. This is the author's deletion mechanism: set an inherited key to `null` in the child to drop it.

`vars` maps merge per-key with the child winning; values replace wholesale.

### `vars` and `${}` substitution

After the whole chain merges, `${var}` placeholders are substituted **once** over the entire merged document using the effective vars. `vars` must be a top-level object; var names match `[A-Za-z_][A-Za-z0-9_]*`; var values are any non-null JSON.

- A string value that is **exactly** `"${name}"` is replaced by the var's raw typed JSON value, preserving type. This is how a quoted YAML string becomes a real integer:

  ```yaml
  vars:
    frames: 5
  steps:
    - wait_frames: "${frames}"   # resolves to the integer 5, not the string "5"
  ```

  Arrays and objects are preserved the same way when a var is embedded via exact match.
- An embedded `${name}` inside a longer string interpolates **scalars only** (string/number/bool), e.g. `res://saves/${slot}/data.tres`. Embedding an array or object into a longer string is rejected with `scenario_vars_invalid`.
- `$${name}` escapes to a literal `${name}` (no substitution).
- Keys are never substituted; only string values, anywhere in the document. Vars cannot reference other vars, and substituted values are final (never re-substituted). Unused vars are fine.
- Any remaining unescaped `${...}` fails with `scenario_var_unresolved`, whose `actual` lists each `{placeholder, location}` with a JSON-pointer-ish location such as `steps[3].tap_action`.

### `abstract: true`

An abstract file is a template, not a runnable scenario. It is excluded from `scenario list` (and therefore from run-by-name, bundle resolution, and lint's duplicate-scenario-name index), so templates may legally lack a `name` or share file stems. Loading or running an abstract file directly fails with `scenario_abstract`. `scenario lint` still validates abstract templates structurally (their `extends`/`vars` blocks), reporting malformed ones as `invalid_scenario_file` while skipping full scenario validation and GQA rules (a template may be intentionally incomplete).

Marking a base `abstract: true` is a non-normative convention: a common layout is a `templates/` subdirectory and/or a `*.base.yaml` suffix, but neither is required — only `abstract: true` controls exclusion.

### Template error kinds

| kind | when |
| --- | --- |
| `scenario_abstract` | an abstract file is loaded or run directly |
| `scenario_extends_missing` | the referenced base file is missing or unreadable |
| `scenario_extends_cycle` | the `extends` chain loops back on itself |
| `scenario_extends_invalid` | `extends` is not a valid relative `.yaml`/`.yml`/`.json` path, or the chain exceeds depth 8 |
| `scenario_vars_invalid` | `vars` is not an object, a var name is invalid, a var value is `null`, or a non-scalar var is embedded in a string |
| `scenario_var_unresolved` | one or more `${...}` placeholders had no matching var |

### Worked example

An abstract base parameterizes the runnable main-menu flow; two concrete children re-tune it. Note `menu_smoke_visual` **replaces** the whole `steps` array to add one step (array-replace, not merge).

```yaml
# templates/menu_smoke.base.yaml
abstract: true
vars:
  wait_frames: 5
  post_click_frames: 10
  save_screenshot: false
scene: res://ui/main_menu.tscn
snapshots:
  save_screenshot: "${save_screenshot}"
steps:
  - wait_frames: "${wait_frames}"
  - snapshot: before
  - click: qa:start_button
  - wait_frames: "${post_click_frames}"
  - snapshot: after_start
assert:
  - visible: qa:pause_menu
  - no_errors: true
```

```yaml
# templates/menu_smoke_default.yaml — name + var overrides only
name: menu_smoke_default
extends: menu_smoke.base.yaml
vars:
  wait_frames: 8
```

```yaml
# templates/menu_smoke_visual.yaml — opt into screenshots + replace the steps array
name: menu_smoke_visual
extends: menu_smoke.base.yaml
vars:
  save_screenshot: true
steps:
  - wait_frames: "${wait_frames}"
  - snapshot: before
  - click: qa:start_button
  - wait_frames: "${post_click_frames}"
  - snapshot: after_start
  - wait_frames: 3
```

There is intentionally no `--var` CLI flag; see [implementation-decisions.md](implementation-decisions.md) for why run-time vars are deferred.

## Launch arguments

Use `godot_args` for Godot engine flags and `project_args` for project/game command-line flags:

```yaml
godot_args:
  - --single-window

project_args:
  - --save-slot
  - qa-profile
  - --seed=42
```

The CLI builds an argv array instead of a shell string. Do not combine arguments into one shell fragment such as `"--save-slot qa-profile"` unless that exact single string is the value the project expects. At launch, the effective order is Godot binary and CLI-owned runner flags, generated QA pacing flags when active, then scenario `godot_args`, then Godot's `--` separator, then scenario `project_args`.

`scenario sweep` (see the CLI spec) reuses this launch contract per seed: it strips any `--seed` already declared in `project_args`, appends `--seed=<s>` as the last project arg, and overwrites `determinism.seed` when the scenario declares one, so the sweep seed always wins for that run while the source document stays unchanged.

Scenarios that intentionally validate project/game flags should set `lint.validates_project_args: true` or `lint.validates_project_flag: true`. `scenario lint` rule `GQA-LINT-008` warns when that metadata is present without non-empty `project_args` or a documented equivalent fixture such as `lint.project_args_fixture` or `lint.equivalent_project_args_fixture`.

## Execution isolation

The optional `execution` block declares how a scenario may be scheduled by the warm bundle runner (`scenario bundle run --warm`, see the CLI spec's warm bundle section):

```yaml
execution:
  isolation: cold
```

`execution.isolation` accepts exactly `warm` or `cold`. Unset (the default) means the scenario is warm-eligible. The warm bundle runner partitions warm-eligible scenarios three ways (see the CLI spec's warm bundle section for the full rules and `data.warm` shape):

- **headless-warm** — no windowed-media need; runs inside the shared headless Godot process.
- **windowed-warm** — its only isolation need is windowed media evidence (a saved screenshot, a visual pixel assertion, a `screenshot_match`, a single-viewport `layout_audit`, or audio capture) and it is otherwise warm-compatible (single viewport, matching `determinism.fixed_fps`, no `godot_args`). It runs inside a shared WINDOWED session that captures real pixels between scene reloads, instead of relaunching Godot per scenario. This is `data.warm.ran_warm: true, mode: "windowed"` — not a fallback.
- **truly-cold** — a structural isolation need (scenario `godot_args`, more than one viewport — which includes a multi-viewport `layout_audit` matrix — or a differing `determinism.fixed_fps`). Falls back to a fresh cold process. A structural reason wins over a media reason. A single-viewport `layout_audit` is windowed-warm, not truly-cold.

`isolation: cold` is the explicit opt-out for scenarios that rely on process-global state that cannot be reset between scenarios — the scenario always gets a fresh Godot process, and because the opt-out is declared, it is exempt from `--warm-strict` enforcement (`data.warm.declared: true` in results). Prefer leaving it unset and fixing the shared state instead; a declared cold scenario permanently costs a full engine boot in every warm bundle run.

This partitioning only matters when the bundle actually runs warm. When a bundle has no more scenarios than parallel lanes (resolved parallelism ≥ runnable scenario count), the runner skips warm entirely and runs the whole bundle on the parallel cold path — there is no session to reuse, so warm would only add overhead (see the CLI spec's warm bundle section, `reason: "no_reuse_below_parallelism"`).

## Frame Pacing And Time Control

`scenario run` advances frames as fast as Godot can process them by default. The CLI launches Godot with generated `--fixed-fps <n>` and `--disable-vsync` flags; `--fixed-fps` defines the simulated frame delta while disabling realtime synchronization. If `determinism.fixed_fps` is present it must be a positive integer and is used as `<n>`. If absent, the CLI derives `<n>` from `project.godot`: first positive `application/run/max_fps`, then `physics/common/physics_ticks_per_second`, then Godot's default `60`. `application/run/max_fps = 0` is ignored for this purpose because it means uncapped max FPS, not a deterministic frame duration.

Generated frame pacing is CLI-owned metadata, not Godot-owned runtime state. `scenario run --json` includes additive `data.frame_pacing` with `mode: "fastest"`, `fixed_fps`, `source`, `project_setting`, and `generated_godot_args`. `scenario profile` follows the same pacing rule; a top-level `time_control` disables generated fastest pacing. A real-time `scenario profile --soak` requires `time_control: {time_scale: 1.0}`, with no `determinism.fixed_fps`. Live sessions are realtime by default and can opt into the same launch-time pacing with `session start --fastest` or live-script `session_start.frame_pacing`.

Scenario acceleration is opt-in. A scenario runs at normal Godot time unless it declares a top-level `time_control` object:

```yaml
time_control:
  time_scale: 8.0
  physics_ticks_per_second: 480
  max_physics_steps_per_frame: 16
```

`time_control.time_scale` maps to Godot `Engine.time_scale` for the duration of one `scenario run` or `scenario profile` invocation. It must be a positive finite number greater than `0.0` and `<= 64`. Zero, negative numbers, strings, booleans, null, NaN, infinity, and values above `64` fail CLI validation before Godot launch with `type: "scenario_validation_failed"`.

`physics_ticks_per_second` maps to `Engine.physics_ticks_per_second`, and `max_physics_steps_per_frame` maps to `Engine.max_physics_steps_per_frame`. They are optional, but when present each must be an integer `>= 1`. Pair both physics fields with high `time_scale` values for physics-sensitive tests so the accelerated run preserves enough physics precision.

Declaring `time_control` disables generated fastest frame pacing for that run. `time_control.time_scale: 1.0` is the persisted opt-out for scenarios that need realtime pacing, while values above `1.0` keep the fixed game-speed behavior for special cases. `time_control` and `determinism.fixed_fps` are mutually exclusive and fail validation if combined.

Acceleration is intended for seconds-based waits, timers, and slow gameplay setup that would otherwise waste wall-clock time. It does not change frame-based wait counts, does not approve screenshot baselines, and does not replace player-facing input or runtime behavior under test. Godot `Engine.time_scale` does not accelerate audio playback speed, so audio-sensitive scenarios should avoid time-control acceleration or treat it as warning-level evidence to review. Screenshot-heavy, visual-risk, audio-sensitive, or physics-precision-sensitive flows that request acceleration without both paired physics settings emit `time_control_physics_pairing_missing` warning evidence under `time_control.warnings` and `data.time_control_warnings` where applicable.

Runtime evidence is additive. When `time_control` is requested, results include `original`, `requested`, `effective`, `restored`, `restored_matches_original`, `restoration_status`, and `warnings`. `restoration_status: "restored"` means the Godot addon restored `Engine.time_scale`, `Engine.physics_ticks_per_second`, and `Engine.max_physics_steps_per_frame` to their original values before final result reporting.

## Progress, Timeouts, And Slow Summaries

Progress evidence is additive and artifact-backed. During `scenario run`, the runner writes `.qa/artifacts/<scenario-name>/<run-id>/progress.jsonl` before and after supported steps and assertions. Events identify the active `step_index` or `assertion_index`, operation kind, safe target or locator context when present, `started_at`, `elapsed_ms`, and run artifact paths. The progress stream is redacted like persisted run metadata: live session tokens and secret-looking `project_args` are not written in clear text.

Scenario timeout fields may be declared at the run level with `timeout_seconds`, `timeout`, `run_timeout_seconds`, or `run_timeout`. Supported duration strings use `ms`, `s`, or `m` suffixes, for example `250ms`, `5s`, or `2m`; bare numeric values are seconds. Step and assertion entries may use `timeout_seconds` or `timeout` for operation-local limits where the runner supports them. Timeout values must be positive finite durations. Zero, negative numbers, non-numbers, empty strings, NaN/infinity, and unsupported strings such as `1hour` or `forever` fail validation with `type: "scenario_validation_failed"` before artifact creation or Godot launch.

Runtime timeout diagnostics use `type: "scenario_run_timeout"` and should include `command: "scenario run"`, active step or assertion index when known, operation kind, redacted target/locator context, elapsed time, configured timeout, expected state, actual state, cleanup evidence for owned Godot processes, and artifact paths. This is the scenario-side counterpart to manifest test `check_test_timeout` failures. A timeout is infrastructure/runtime failure evidence, not a gameplay assertion failure.

When a run timeout is declared and frame pacing is active (no `time_control`), the declared timeout is enforced by the engine as a load-independent frame budget instead of a wall clock: the CLI writes `run.frame_budget` (`frames = ceil(timeout_seconds x fixed_fps)`, plus `timeout_seconds`, `fixed_fps`, and the pacing `source`) into `request.json`, and the runner fails the scenario with `type: "scenario_frame_budget_exceeded"` once the budget is consumed, checked at step and assertion boundaries with early cutoffs inside `wait_frames`, `wait_until`, and `drive_policy` loops. The run exits cleanly with a normal `result.json` and `timed_out: false`; overshoot inside a single non-frame step or assertion is bounded by that operation's own parameters. Because `--fixed-fps` runs frames at CPU speed, host load cannot fail a correct scenario under a frame budget. Process and warm-transport wall clocks then act only as hang backstops: the declared timeout with 3x slack, capped at 1800 seconds, and never below the declared value. Scenarios with `time_control` or without a declared run timeout keep pure wall-clock behavior (`scenario_run_timeout`).

Slow summaries are informational. `slow_summary_threshold_ms` and `slow_summary.threshold_ms` configure the threshold for `.qa/artifacts/<scenario-name>/<run-id>/slow-summary.json`. The artifact includes the slowest completed operations from progress evidence, usually under `slow_operations`, sorted by elapsed time and bounded to ten entries. A fixture such as `slow_step_progress_demo` can lower the threshold to prove evidence generation. Slow thresholds do not fail successful runs.

## Step types

```yaml
- wait_frames: 5
- wait_seconds: 0.2
- wait_until:
    source: method
    target: qa:main_menu
    method: get_qa_state
    path: status
    operator: equals
    expected: ready
    timeout_frames: 120
    poll_every_frames: 10
- snapshot: before
- tap_action: ui_accept
- hold_action:
    action: ui_down
    frames: 4
- release_action: ui_down
- input_joy_button:
    button: start
    device: 2
    frames: 2
- input_joy_motion:
    axis: left_x
    value: 1.0
    device: 2
    frames: 2
- input_joy_state:
    inputs:
      - {device: 0, button: {name: a, pressed: true}}
      - {device: 3, button: {name: x, pressed: true}}
      - {device: 3, motion: {axis: left_x, value: -1.0}}
    frames: 30
- drive_policy:
    policy: crew_bot
    params: {skill: competent}
    devices: [0, 1]
    max_frames: 3600
    tick_every_frames: 6
    until:
      source: provider
      provider: match_state
      path: room.completed
      operator: equals
      expected: true
- click: qa:start_button
- teleport_node:
    target: qa:player
    to: qa:test_island_landing
- fixture_teleport:
    target: qa:player
    to: qa:test_island_landing
- spawn_scene:
    scene: res://fixtures/loot.tscn
    parent: qa:arena
    qa_id: test_loot
    at: qa:loot_anchor
- set_property:
    target: qa:team_1
    property: scrap
    value: 10
- joy_connect:
    devices: [0, 1, 2, 3]
    name: QA Virtual Pad
- joy_disconnect:
    devices: [3]
- call_method:
    target: qa:island_manager
    method: qa_spawn_test_loot
    args: [cannon]
- record_state:
    name: match_summary
    source: provider
    provider: match_state
    path: summary
- assert:
    - visible: qa:player
    - text_contains: {target: qa:status, expected: ready}
- set_state:
    key: save_profile
    value: empty
```

`snapshot` names become artifact path segments under `snapshots/<snapshot-name>/`, so they must use only letters, numbers, `.`, `_`, and `-`.
`tap_action` and `release_action` take a single action name string. `hold_action` takes an object so the scenario can make the hold duration explicit without inventing a separate wait step. `input_joy_button` accepts a button string shorthand or an object with explicit `device` and `frames`; `input_joy_motion` always uses an object with `axis`, `value`, and optional `device`/`frames`, matching the live `input joy` command semantics. `input_joy_state` is open-loop batch joypad input for concurrent multi-device holds; see "Concurrent joypad state (`input_joy_state` step)" below. `drive_policy` is closed-loop joypad input computed by a registered policy each tick; see "Policy-driven input" below. `click` and `node_inspect` take a single locator string and resolve it with the same runtime locator rules used by assertions. `node_inspect` records bounded node evidence under `node_inspections` in `result.json` and command JSON, keyed by step index plus target. `record_state` is a read-only observation step that captures a state VALUE as run evidence under `state_records`; see "Recording state evidence (`record_state` step)" below. `joy_connect` and `joy_disconnect` are setup-class steps that simulate joypad hardware connection state by firing the engine's `joy_connection_changed` signal so a local-multiplayer lobby's connect/disconnect handlers run, with a `QaRuntime.effective_connected_joypads()` shim for lobbies that poll the roster instead (a documented Godot 4.x limitation means `Input.get_connected_joypads()` itself cannot be faked); see "Joypad connection simulation (`joy_connect` / `joy_disconnect` steps)" below. Step-local `assert` runs the listed assertions immediately at that point in the scenario; top-level `assert` still runs after all steps.

`click` targets a Control in the active root Window. Its local center is converted
to window input coordinates using current canvas/content scaling and letterbox
transforms, so a resized or scaled viewport still exercises normal mouse input.
Targets inside a SubViewport or a different Window fail as `target_not_clickable`
with expected/actual viewport paths. This limitation does not affect keyboard or
action input steps.
Runtime `qa:` locators match a node's `qa_id`, `qa.id.<id>` or `qa.<id>` groups, and `qa_aliases` metadata. Alias metadata may contain either raw ids such as `player` or prefixed locators such as `qa:player`; duplicate alias/id matches on the same node count once.
Setup steps are for deterministic fixtures, not for faking the behavior under test:

- `teleport_node` moves a `Node2D`, `Control`, or `Node3D` to another locator's global position (`to`), an explicit `{x, y, z?}` position, or a `to_point` provider result. It resets transient velocity on supported physics bodies by default; set `preserve_velocity: true` only when the scenario needs existing motion to continue.
- `fixture_teleport` is a narrower setup alias for fixture placement. It supports the same destination forms as `teleport_node` and is intended for deterministic starts before player-facing actions.
- `spawn_scene` instantiates a `res://` `PackedScene`, parents it under the scene root or `parent`, optionally assigns `name` and root `qa_id`, and optionally places it at `at` or `position`.
- `set_property` sets a property on a resolved node using a JSON value. Typed dictionaries with `type: Vector2`, `Vector2i`, `Vector3`, or `Color` are decoded to Godot values.
- `call_method` calls only project-defined fixture methods whose names begin with `qa_`; optional `args` are JSON-compatible and use the same typed dictionary decoding as `set_property`.
- `fixture_call` calls a registered fixture driver by `driver` and `action`; `params` defaults to `{}` and must be JSON-compatible. Fixture drivers are for deterministic setup/mutation only and are not callable through assertions or read-only state paths.
- `joy_connect` / `joy_disconnect` announce virtual joypads as connected/disconnected by firing the engine's `joy_connection_changed` signal, so a lobby's signal-driven connect/disconnect handlers run; polling lobbies adopt the `QaRuntime.effective_connected_joypads()` shim instead. See "Joypad connection simulation (`joy_connect` / `joy_disconnect` steps)" below for the full contract, cleanup, the real-pad disconnect guard, and the Godot 4.x limitation (`Input.get_connected_joypads()` is not faked).

Point providers let scenarios request logical world-space coordinates without requiring generated cells, stations, island areas, or pickup helpers to carry `qa_id` metadata. Use `to_point` with a durable provider locator, a QA-prefixed read-only method, and an opaque logical id:

```yaml
steps:
  - fixture_teleport:
      target: qa:player
      to_point:
        provider: qa:team_1_raft
        method: qa_point_probe
        id: station:motor
      offset: {x: 3.0, y: 0.0}
```

The provider method must return JSON-compatible world-space point data:

```json
{"ok": true, "coordinate_space": "world_2d", "point": {"x": 0.0, "y": 0.0}, "logical_id": "station:motor"}
```

Distance assertions can use the same provider object with `a_point`/`b_point` instead of `a`/`b` locators.

Prefer durable anchors plus project-owned adapters for dynamic gameplay setup and observation. The scenario format does not require every generated object or transient value to have its own `qa_id`; a stable manager/actor anchor can expose QA-prefixed contracts such as `get_qa_inventory_state()`, `get_raft_motion_qa_state()`, or `qa_collision_probe()` for runtime code to inspect. Use runtime inspection/reflection first for engine-visible facts, small registered read-only providers for semantic aggregate state, registered probe providers for bounded surface/spatial queries, and separate registered fixture drivers for deterministic setup. Legacy QA-prefixed methods remain supported, but new projects should avoid making QA adapter layers the production scene entry point.

Profiling does not require new scenario syntax. `godot-qa scenario profile <name> --project <project> --json` runs an existing scenario with runtime profiling enabled, records frame/monitor samples and per-step timing, and leaves the scenario's normal assertion thresholds and pass/fail behavior unchanged.

```yaml
steps:
  - call_method:
      target: qa:inventory_system
      method: qa_grant_test_item
      args: [repair_kit]
  - snapshot: after_grant

assert:
  - visible: qa:inventory_panel
  - text_contains:
      target: qa:inventory_summary
      expected: Repair Kit
  - state_matches:
      target: qa:main_menu
      source: method
      method: get_raft_motion_qa_state
      matches:
    - {path: mode, operator: equals, expected: steady}
    - {path: speed, operator: approx_equals, expected: 4.2, epsilon: 0.001}
    - {path: distance, operator: greater_than, expected: 12.0}
        - {path: actor_surface, operator: present}
  - no_errors: true
```

Numeric checks support the `within` operator for absolute-tolerance windows: `|actual - expected| <= tolerance`. Unlike `approx_equals` (whose `epsilon` defaults to `0.0001` and exists for float noise), `within` requires an explicit positive `tolerance` and is meant for gameplay-sized windows ("temperature is 100 ± 5"):

```yaml
assert:
  - state_matches:
      target: qa:main_menu
      source: method
      method: get_raft_motion_qa_state
      path: motion.position.y
      operator: within
      expected: 420
      tolerance: 1
```

`within` is available everywhere the state-match operator vocabulary is accepted: `state_matches`, `wait_until`, `drive_policy.until`, and `observe_until` property conditions. A missing, zero, negative, or non-numeric `tolerance` fails validation before launch.

Aggregate state methods are best for compact gameplay summaries on durable anchors, such as scoreboards, raft motion, objectives, inventories, or actor lists. Registered state providers are the preferred new contract for those summaries:

```yaml
assert:
  - state_matches:
      source: provider
      provider: race_feel
      path: phase
      operator: equals
      expected: gameplay
```

Provider IDs are separate from `qa:` node locators. `provider: race_feel` can read state, but it cannot be used for click, visibility, rect, or focus commands. Registered provider methods do not need `qa_` naming because explicit registration is the permission boundary; they must still be read-only, JSON-compatible, bounded, and non-mutating by behavior. Legacy aggregate methods must be zero-argument, read-only by convention, JSON-compatible, and named with `qa_*`, `get_qa_*`, or `get_*_qa_state`.

Use setup `call_method` only for deterministic fixture mutation, probes for bounded spatial/surface queries, visual assertions for rendered perception, and debug-label text assertions only when the visible text itself is the player-facing contract. Prefer registered fixture drivers for new deterministic setup.

`qa-state validate` complements `state_matches` for registered providers, project-owned aggregate state methods, and saved artifacts; it does not replace scenario assertions. Use `state_matches` inside scenarios for concrete runtime assertions, and use schemas when a team wants a stable review playbook for the shape and allowed values of provider output, legacy `get_qa_*_state()` / `get_*_qa_state()` output, saved JSON fixtures, or trace state artifacts. Schemas are optional: the scenario format does not require every project or every QA method to define one, and schema validation is not a reason to add hidden debug labels or per-object `qa_id` tags.

`state_matches` can select exactly one item from an aggregate array before checking a nested value:

```yaml
assert:
  - state_matches:
      target: qa:team_1_raft
      source: method
      method: get_qa_raft_grid_state
      path: modules
      where: {module_id: cannon}
      value_path: health
      operator: equals
      expected: 75
```

Array selectors fail clearly when the path is not an array, `where` is not an object, `value_path` is missing, there are zero matches, or more than one item matches.

For spatial and rendered facts, prefer project-owned probes and screenshot-backed assertions instead of polling hidden debug labels:

```yaml
steps:
  - call_method:
      target: qa:arena_controller
      method: qa_place_actor_for_surface_probe
      args: [{type: Vector2, x: 120, y: 420}]
  - wait_frames: 2
  - snapshot: after_surface_setup

assert:
  - surface_probe:
      target: qa:player
      provider: qa:arena_controller
      provider_method: qa_surface_probe
      coordinate_space: world_2d
      surfaces: [walkable, blocked]
      points:
        - {x: 120, y: 420, expected: walkable}
  - surface_matches:
      target: qa:player
      surface: walkable
      coordinate_space: world_2d
      provider: qa:arena_controller
      provider_method: qa_surface_probe
      actor_surface_method: qa_current_surface
  - visible_pixels:
      target: qa:player
      snapshot: after_surface_setup
      min_ratio: 0.9
  - saved_snapshot_pixels:
      target: qa:player
      snapshot: after_surface_setup
      min_count: 25
  - not_occluded:
      target: qa:player
      snapshot: after_surface_setup
```

Use names like `get_qa_*_state()`, `get_*_qa_state()`, and `qa_*_probe()` as project-owned method/probe contracts, not as new scenario assertion names. Debug labels remain useful diagnostics, but scenario assertions should read them with `text_contains` or `text_equals` only when that exact text is part of the player-facing contract.

Runtime QA collision providers can expose explicit collision-feel state for live diagnostics such as `physics collision-report`:

```json
{
  "contacts": [
    {
      "contact_id": "wall_a:shape_0",
      "other_id": "wall_a",
      "surface_type": "hard",
      "normal": {"x": 1, "y": 0},
      "depth": 3.5,
      "relative_velocity": {"x": -120, "y": 12},
      "is_soft": false
    },
    {
      "contact_id": "field_trigger",
      "other_id": "assist_field",
      "surface_type": "soft",
      "is_soft": true
    },
    {
      "contact_id": "goal_sensor",
      "other_id": "goal_area",
      "surface_type": "trigger"
    }
  ],
  "velocity": {"x": 0, "y": -10},
  "angular_velocity": 0.2,
  "progress_scalar": 123.0,
  "rescue_push_count": 0
}
```

Soft-contact warnings require this explicit provider classification. The runtime and CLI must not infer `hard`, `soft`, or `trigger` from node names such as wall, buoy, rock, sensor, or trigger.

## Recorder candidates

`scenario record` currently generates replayable JSON candidates with a conservative review block:

```json
{
  "name": "main_menu_recording",
  "scene": "res://ui/main_menu.tscn",
  "steps": [
    {"wait_frames": 5},
    {"snapshot": "initial"},
    {"click": "qa:start_button"},
    {"snapshot": "final"}
  ],
  "assert": [],
  "review": {
    "status": "needs_review",
    "generated_by": "scenario record",
    "artifacts": {
      "recording": ".qa/artifacts/recordings/main_menu_recording/20260421T120000Z/recording.json",
      "candidate": ".qa/artifacts/recordings/main_menu_recording/20260421T120000Z/candidate.json"
    },
    "suggested_assertions": [
      {
        "kind": "negative_visibility_guard",
        "assertion": {"hidden": "qa:invalid_context_toast"},
        "input_moment": "input:ui_accept@frame:22",
        "target": "qa:invalid_context_toast",
        "snapshots": ["initial", "moment-22", "final"],
        "evidence": {
          "runtime": "toast remained absent while invalid input was replayed",
          "artifacts": [
            ".qa/artifacts/recordings/main_menu_recording/20260421T120000Z/snapshots/moment-22/ui.json"
          ]
        }
      }
    ],
    "unconverted_events": []
  }
}
```

Recorded gamepad input replays through the standard `input_joy_button` / `input_joy_motion` steps: taps use the shorthand string form, holds keep `frames` equal to the observed press span, analog motion is quantized (bucket step, deadzone snap-to-zero, minimum per-axis frame gap — configurable via the `scenario record` `--joy-motion-*` flags), and frame gaps between joypad events become interleaved `wait_frames` steps so the candidate preserves the human's timeline.

Recorder candidates intentionally keep `assert` empty by default. Reviewers are expected to inspect `.qa/artifacts/recordings/<name>/<run-id>/recording.json` plus the saved snapshots, then manually accept, reject, or edit each `review.suggested_assertions` entry before copying accepted assertions into `assert` and rerunning `scenario run <name> --json`.

## Assertion types

```yaml
- exists: qa:pause_menu
- not_exists: qa:loading_spinner
- visible: qa:pause_menu
- hidden: qa:main_menu
- enabled: qa:start_button
- disabled: qa:apply_button
- focused: qa:resume_button
- text_equals:
    target: qa:title
    expected: Settings
- text_contains:
    target: qa:error_message
    expected: Not enough coins
- inside_viewport: qa:settings_panel
- no_overlap:
    a: qa:start_button
    b: qa:settings_button
- focus_path:
    start: qa:start_button
    actions: [ui_down, ui_down]
    expected: [qa:settings_button, qa:quit_button]
- screenshot_match:
    snapshot: after_start
    baseline: qa/baselines/example.png
    threshold: 0.01
- no_errors: true
```

Implemented through S9: `exists`, `not_exists`, `visible`, `hidden`, `enabled`, `disabled`, `focused`, `text_equals`, `text_contains`, `inside_viewport`, `no_overlap`, `no_errors`, `layout_audit`, and `screenshot_match`.
`layout_audit` currently accepts `default` and `strict`. `focus_path` remains a future contract example.

`layout_audit` is a CLI-side post-process over a saved snapshot, so the run must capture one (add a `snapshot` step) — a `layout_audit` run that records no snapshot fails with `layout_audit_missing_snapshot`. With zero or one `viewports` entry the audit runs on the normal single run and its geometry failures attach inline to the ordinary single-run result envelope (a lone `viewports` entry just sets the window resolution); `layout_audit` and `screenshot_match` can both apply to that single run. With two or more `viewports` entries the run fans out into the multi-viewport matrix envelope (`data.viewports` + `data.runs[]`), auditing each cell.

## Waiting on runtime state (`wait_until` step)

`wait_until` is a step (not an assertion) that blocks the scenario until a state predicate holds or a bounded frame budget elapses. Use it instead of chaining fixed `wait_frames` guesses when the real gate is a gameplay/UI state transition. It reuses the exact `state_matches` state-reading machinery and operator vocabulary, so the predicate you can express in a `state_matches` assertion you can also wait on.

```yaml
steps:
  - click: qa:start_button
  - wait_until:
      source: method
      target: qa:main_menu
      method: get_qa_state
      path: status
      operator: equals
      expected: ready
      timeout_frames: 120
      poll_every_frames: 10
  - snapshot: ready

assert:
  - visible: qa:pause_menu
  - no_errors: true
```

Fields:

- `source`: required, either `method` (read a QA-prefixed read-only method on `target`) or `provider` (read a registered state provider). This is the state source type, mirroring `state_matches`.
- `target`: required when `source: method`; the durable locator whose method is called.
- `method`: required when `source: method`; a `qa_*` / `get_qa_*` / `get_*_qa_state` read-only method returning JSON-compatible data.
- `provider`: required when `source: provider`; a registered state-provider id.
- `path`: required non-empty dot-separated state path such as `status` or `motion.state`. Optional `where` + `value_path` select one item from an aggregate array before reading, exactly as in `state_matches`.
- `operator`: required, one of `equals`, `contains`, `present`, `absent`, `approx_equals`, `approximately_equals`, `greater_than`, `less_than`, `within`.
- `expected`: required for value operators (`equals`, `contains`, `approx_equals`, `approximately_equals`, `greater_than`, `less_than`, `within`); omit it for `present`/`absent`. `epsilon` is optional for approximate comparisons (default `0.0001`). `tolerance` is required (positive number) for `within` and asserts `|actual - expected| <= tolerance`.
- `timeout_frames`: required positive integer frame budget.
- `poll_every_frames`: optional positive integer polling cadence; defaults to `10`.

The step polls the predicate immediately and then every `poll_every_frames` frames until it passes or `timeout_frames` elapses. On success the scenario continues. On timeout it fails the scenario with `type: "wait_until_timeout"`, including the configured predicate and the last observed state under `last_observed`. Unresolved targets/sources, invalid methods, and non-JSON-compatible results fail immediately with their underlying error type rather than waiting out the full budget. Cold and warm execution share the same runner, so `wait_until` behaves identically in both.

## Recording state evidence (`record_state` step)

`record_state` is a read-only observation step: instead of asserting on runtime state (like `state_matches`) or waiting for it (like `wait_until`), it *reads a state value once and records it as run evidence*. Use it when a scenario needs to capture a value per run rather than gate on it — for example capturing a match-summary provider's result so a `scenario sweep` across a seed range surfaces each seed's outcome and win-rates become a single read of `sweep.json`. Aggregation and analytics stay out of the tool: `record_state` and the sweep only carry the raw per-run/per-seed values; the agent reads them.

```yaml
- record_state:
    name: match_summary          # required: unique artifact key, snapshot-name charset
    source: provider             # provider | method — same vocabulary as wait_until/state_matches
    provider: match_state        # source: provider
    # or: source: method, target: qa:game, method: get_qa_match_state
    path: summary                # optional: dot-separated selection into the read value
```

- `name`: required, must be a safe artifact segment (letters, numbers, `.`, `_`, `-`, same as a `snapshot` name) and unique across the scenario's `record_state` steps — the name keys the `state_records` evidence map, so a collision is rejected during validation to avoid silently overwriting an earlier record.
- `source`/`provider`/`target`/`method`: reuse the EXACT state-source read path `wait_until` and `state_matches` use. `source: provider` reads a registered state provider; `source: method` reads a read-only QA method (`qa_*`, `get_qa_*`, or `get_*_qa_state`, zero-argument) off a resolved `target`. The same provider/method allowlist and read-only rules apply automatically.
- `path`: optional non-empty dot-separated selection into the read value (same selection helper state checks use). When present the recorded value is the selected sub-value and `present` reports whether the path resolved; when the path is absent from the value the record captures `null` with `present: false` (an absent selection is evidence, not a failure). Omit `path` to record the whole value.
- Evidence: on success the value is recorded under a top-level `state_records` map in `result.json` (and lifted into `scenario run` command JSON), keyed by `name`. Each entry carries `step_index`, `name`, the source descriptor (`source` plus `provider` or `target`/`resolved`/`method`), `path`, `present`, the bounded `value`, and `value_bytes`. `scenario sweep` includes each child run's `state_records` in its `per_seed[]` entries (null when a seed recorded nothing).
- Bounded: the value comes from one controlled read (already required JSON-compatible) and its serialized form must stay within a fixed byte cap; an over-cap value fails the step with `type: "state_record_value_too_large"` (a truncated value would be corrupt JSON) rather than being silently trimmed.
- Failure modes: an unresolved provider/target, an invalid/mutating/non-zero-arg method, or a non-JSON-compatible value fail the step immediately with the same underlying error types the `state_matches`/`wait_until` state-source path produces; an over-cap value fails with `state_record_value_too_large`. Like `node_inspect`, `record_state` is a read-only observation — it is neither player input nor a fixture/state mutation, so `GQA-LINT-001` never flags it or treats it as an ordering marker. Cold and warm execution share the same runner, so `record_state` behaves identically in both.

## Joypad connection simulation (`joy_connect` / `joy_disconnect` steps)

`joy_connect` and `joy_disconnect` are setup-class steps that simulate joypad *hardware connection* state — not player input. Injected joypad events (`input_joy_button`, `input_joy_motion`, `input_joy_state`, `drive_policy`) work regardless of connection state, but a local-multiplayer lobby that listens to the `joy_connection_changed` signal (spawn a player on connect, pause on drop) never sees any pads under this tool (headless, or windowed without physical controllers). That makes signal-driven "press A to join" / N-player join flows and mid-match controller-drop handling (pause-on-drop, the classic couch-game bug class) unvalidatable. These steps fire the engine's `Input.joy_connection_changed(device, connected)` **signal** — the same signal the platform layer emits on a real connect/disconnect — so those listeners run.

```yaml
- joy_connect:
    devices: [0, 1, 2, 3, 4, 5]
    name: QA Virtual Pad        # optional, default "QA Virtual Pad"
- joy_disconnect:
    devices: [3]
```

- `devices`: required non-empty list of unique non-negative integers. Duplicate or negative entries are rejected during validation.
- `name` (`joy_connect` only): optional non-empty string, default `"QA Virtual Pad"`. It is echoed in the step result as QA metadata. See the engine limitation below: on Godot 4.x the `joy_connection_changed` signal carries only `(device, connected)`, so `name` is *not* delivered to the engine and does not change `Input.get_joy_name(device)`.
- Batch/settle: all devices in one step are announced back-to-back, then the step waits exactly one delivery frame so the `joy_connection_changed` listeners settle before the step returns.
- Idempotency (connect): re-announcing an already-faked device does not re-fire the `connected` signal (so a listener cannot double-join); the request still succeeds and the device stays tracked.
- Real-pad guard (disconnect): `joy_disconnect` only disconnects devices previously faked via `joy_connect`. A request that names any device the tool did not fake fails with `invalid_params` (`command: setup.joy_disconnect`, `actual`: the offending devices, `faked_devices`: the current fake set) and fires **nothing** — this prevents emitting a spurious disconnect for a device index that could correspond to a real physical pad and wrongly pause the game / drop a real player. Disconnecting a faked device removes it from tracking.
- Success payload echoes `devices` (the normalized list acted on), `name` (connect only), `announced_joypads` (the sorted set of QA-announced-connected devices — the observable to assert on), and `connected_joypads` (the real `Input.get_connected_joypads()` roster, for transparency; unaffected on this engine — see below).
- Cleanup: the QA-announced set lives in a process-global tracking dictionary that survives a scene reload, so at scenario finalize, on `input.reset`, and on scene change (`session.change_scene`) the tool fires a matching *disconnect* signal for every still-announced device and clears the set. Warm sessions run through the same shared executor finalize and the same `reset_input()` path, so a faked connection never leaks into the next scenario sharing the process.
- Lint: `joy_connect`/`joy_disconnect` are setup, not player input — like `spawn_scene`/`fixture_call` they are neither the interactive marker nor a mutation for `GQA-LINT-001`. Connecting pads normally happens before input, but placing a `joy_disconnect` *after* input is the canonical mid-match controller-drop scenario, and it is deliberately NOT flagged: `GQA-LINT-001` only flags the four state-mutation steps (`fixture_teleport`, `teleport_node`, `set_property`, `call_method`) after player input, and these joypad-connection steps are intentionally excluded from that set.

### Engine roster limitation: signal-only synthetic connections

The addon emits the `Input.joy_connection_changed` **signal**: the roster-mutating C++ entry point `Input::joy_connection_changed(device, connected, name, guid)` is not exposed to GDScript (`Input.has_method("joy_connection_changed")` is `false`, and `ClassDB.class_has_method("Input", "joy_connection_changed")` is `false`), and the signal itself carries only `(device, connected)`. Consequences:

- These steps fire the `joy_connection_changed` signal (verified to invoke connected handlers under `--headless`), so a lobby that **listens to the signal** to add/remove players is fully exercisable.
- They do **not** add the device to `Input.get_connected_joypads()`, and do not affect `Input.get_joy_name(device)`. A lobby that **polls `get_connected_joypads()`** directly cannot be faked on this engine version — adopt the roster shim below.
- `name` is accepted and echoed for step-contract stability and forward compatibility (if a future engine binds the four-argument method), but it is QA metadata only on 4.x.

### Polling lobbies: the `QaRuntime.effective_connected_joypads()` roster shim

For enumeration-based lobbies, the addon ships a game-facing shim — the same pattern as `QaArgs.effective_user_args()`, the warm-session-aware replacement for `OS.get_cmdline_user_args()`. `QaRuntime.effective_connected_joypads() -> Array[int]` (static, read-only) returns the sorted, deduped **union** of the real `Input.get_connected_joypads()` roster and the devices currently faked via `joy_connect`. With no fakes active it degrades to exactly the engine roster, so real-hardware behaviour is unchanged and a game can adopt it unconditionally. Opt-in is a one-line change at the single read site:

```gdscript
const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")

# Input.get_connected_joypads()  ->  QaRuntime.effective_connected_joypads()
for device in QaRuntime.effective_connected_joypads():
    add_lobby_slot(device)
```

Use this shim to test enumeration-based lobbies with synthetic devices. Direct `Input.get_connected_joypads()` polling does not observe the addon's synthetic roster. `examples/sample_project/ui/fixtures/live_input_probe.gd` demonstrates both paths — its `get_qa_joypad_roster` reports the signal-tracked roster, the shim's `effective_roster_count`, and the raw `engine_roster_count` (which stays `0` under headless QA, making the limitation visible).

## Concurrent joypad state (`input_joy_state` step)

`input_joy_state` is open-loop batch joypad input: it applies many devices' inputs in the *same* frame and lets button holds and axis positions persist across later steps. Use it when a scenario needs concurrent input that the atomic steps cannot express — for example holding a button on device 0 and a different button on device 3 at the same time, since `input_joy_button` is a self-contained press→wait→release and cannot overlap another device's hold open-loop.

```yaml
- input_joy_state:
    inputs:
      - {device: 0, button: {name: a, pressed: true}}
      - {device: 3, button: {name: x, pressed: true}}
      - {device: 3, motion: {axis: left_x, value: -1.0}}
    frames: 30
```

- `inputs`: required, a non-empty list. Each entry is `{device, motion|button}`: a non-negative integer `device` plus exactly one of `motion` (`{axis, value}` — `axis` named like `input_joy_motion`, `value` from `-1.0` to `1.0`) or `button` (`{name, pressed}` — `name` named like `input_joy_button`, boolean `pressed`). This is the exact per-entry shape a `drive_policy` policy returns, but with **no device allowlist**: any non-negative device is accepted.
- `frames`: optional non-negative integer, default `1` — frames to wait *after* emitting the whole batch. `0` emits and continues in the same frame.
- Semantics: the entire `inputs` list is validated before anything is emitted, so a malformed entry fails the step and injects nothing. Valid entries are then emitted back-to-back with no frame awaits between them, so all devices' inputs land on one frame. Buttons stay pressed and axes stay latched until a later step changes them (send `pressed: false` or `value: 0.0`, or another input step). Held buttons and non-zero axes are released automatically at scenario finalize, by `input.reset`, and on scene change — the step adds no cleanup of its own.
- Like the other joypad steps, `input_joy_state` counts as player-facing input for lint (GQA-LINT-001 treats mutations placed after it as ordering violations and never flags the step itself).

## Policy-driven input (`drive_policy` step)

`drive_policy` is a closed-loop input step: instead of replaying a fixed input script, it calls a policy each tick (read state → return desired inputs) and emits those inputs through the same per-device joypad event paths as `input_joy_motion` and `input_joy_button`. Use it when a scenario needs to *play* a mechanic — steer a drone to a socket, chase a moving objective, hold a formation — rather than hope a canned open-loop input sequence still lines up with runtime state.

```yaml
steps:
  - drive_policy:
      policy: crew_bot                 # registered policy provider id, or a builtin
      params: {skill: competent}       # optional, passed through to the policy each tick
      devices: [0, 1]                  # devices the policy may drive; required, non-empty
      max_frames: 3600                 # hard frame budget; required
      tick_every_frames: 6             # decision cadence; defaults to 6 (10 Hz at 60 fps)
      until:                           # optional early-stop predicate
        source: provider
        provider: match_state
        path: room.completed
        operator: equals
        expected: true
```

Fields:

- `policy`: required non-empty id of a registered policy provider, or a builtin policy name such as `builtin:steer_toward`.
- `devices`: required non-empty list of non-negative device ids the policy may drive. Inputs the policy returns for any other device fail the step.
- `max_frames`: required positive integer hard frame budget for the whole step.
- `tick_every_frames`: optional positive integer decision cadence; defaults to `6`.
- `params`: optional object passed through to the policy unmodified on every tick.
- `until`: optional early-stop predicate using the exact `wait_until` state-reading fields (`source`, `provider`/`target`+`method`, `path`, `operator`, `expected`, optional `epsilon`/`tolerance`/`where`/`value_path`). It must not carry `timeout_frames`; the step's `max_frames` is the budget. Checked immediately on step entry and then at the tick cadence.

Each tick the runner checks `until` (when present), then calls the policy with a read-only request dictionary `{policy, tick, frame, devices, params}`. The policy returns the inputs it wants active:

```json
{"inputs": [
  {"device": 0, "motion": {"axis": "left_x", "value": 0.8}},
  {"device": 0, "button": {"name": "a", "pressed": true}}
]}
```

Each entry uses `device` plus exactly one of `motion` (`axis` named like `input_joy_motion`, `value` from `-1.0` to `1.0`) or `button` (`name` named like `input_joy_button`, boolean `pressed`). Axes keep their last value between ticks and buttons stay pressed until the policy changes them, so a policy only needs to return what should change or stay held. The whole list is validated before anything is emitted: a malformed policy result, an unknown policy, or an input outside `devices` fails the step immediately with a clear error and injects nothing on that tick.

The step stops when `until` passes (`stop_reason: "until_satisfied"`) or the budget elapses (`stop_reason: "max_frames"`). Exhausting `max_frames` is a normal stop, not a failure — `until` is an early exit, so assert the required outcome with assertions or a following `wait_until` when the scenario demands it. On every stop (including failures) the runner zeroes all axes it touched and releases all buttons it pressed on the driven devices; scenario finalize additionally resets all injected input as usual. `result.json` records per-step evidence under `policy_drives` keyed by step index: `{policy, devices, ticks, frames, inputs_applied, stop_reason}`.

Integrity contract: the runner only ever *emits input* on the policy's behalf — it never touches node state, so `drive_policy` counts as player-facing input (GQA-LINT-001 flags mutations placed after it and never flags the step itself). Whether the policy reads only player-visible facts is the project's responsibility: policy providers are registered as read-only consumers (arity-1 method, mutating method names rejected), and games should back them with the same read-only state providers / `qa_` state methods a player-visible HUD would use. A policy that steers using hidden truth is a fixture in disguise — prefer exposing a player-visible state provider for policies to consume.

Register a policy from project code, mirroring fixture drivers:

```gdscript
const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")

func _ready() -> void:
    QaRuntime.register_policy_provider("crew_bot", self, "crew_bot_inputs", {"skill_levels": ["novice", "competent"]})

func crew_bot_inputs(request: Dictionary) -> Dictionary:
    # request: {policy, tick, frame, devices, params}
    return {"inputs": [{"device": int(request.devices[0]), "motion": {"axis": "left_x", "value": 1.0}}]}
```

One builtin policy ships with the addon so the step is usable without game code:

- `builtin:steer_toward` — params `{node, target, stick?, tolerance?}`. Each tick it reads the steered `node` and `target` locators (both must resolve to `Node2D`s) and returns normalized stick motion from the node toward the target on the first driven device; within `tolerance` (default `8.0` pixels) it returns zeroed motion so the node stops. `stick` selects the axis pair, `left` (default) or `right`. It is intentionally minimal — smoke tests and simple steering only.

See `examples/sample_project/qa/scenarios/examples/drive_policy_steer_demo.json` for a full closed-loop run that steers an input-driven actor with the builtin policy and then home again with a registered one.

### Team-vs-team matchups

Validating a couch game's balance — team A "aggressive" versus team B "defensive" across many seeds — needs several devices driven by *different* behaviors in the same run. The contract does not grow a per-team or per-player assignment surface for this: `drive_policy` stays **one policy + `devices` + `params`**. Register ONE meta-policy and pass the roster-to-style mapping through `params.teams`; the policy dispatches a per-device brain each tick and returns inputs for every device in a single result:

```yaml
- drive_policy:
    policy: team_matchup
    devices: [0, 1, 2, 3]
    max_frames: 600
    params:
      teams:
        a: {devices: [0, 1], style: rush}
        b: {devices: [2, 3], style: hold}
    until:
      source: method
      target: qa:team_matchup_demo
      method: get_qa_team_matchup_state
      path: decided
      operator: equals
      expected: true
```

Per-team and per-player dispatch is game-side by design. `docs/vision.md` scopes the tool *out* of general game automation — botting frameworks, engine-agnostic drivers, gameplay scripting — and `drive_policy` is the one bounded exception: validation-scoped input generation inside the existing scenario/assertion contract. First-class per-team assignments in the runner would push it back across that line, so team composition and the per-device brains stay in the game's registered policy, which reads `request.params.teams` and dispatches:

```gdscript
func team_matchup_inputs(request: Dictionary) -> Dictionary:
    var teams: Dictionary = request.get("params", {}).get("teams", {})
    var inputs: Array = []
    for team_key in teams:
        var team: Dictionary = teams[team_key]
        var style := str(team.get("style", "rush"))
        for raw_device in team.get("devices", []):
            var device := int(raw_device)
            var target := _objective if style == "rush" else _home_post(device)
            var direction := (target - _actor(device).global_position).normalized()
            inputs.append({"device": device, "motion": {"axis": "left_x", "value": clampf(direction.x, -1.0, 1.0)}})
            inputs.append({"device": device, "motion": {"axis": "left_y", "value": clampf(direction.y, -1.0, 1.0)}})
    return {"inputs": inputs}
```

The meta-policy stays a read-only consumer (arity-1, non-mutating name); the runner only emits the per-device input it returns and never touches node state. Pair it with `record_state` and `scenario sweep` to close the balance-testing loop: one `drive_policy` step plays the match, an `until` predicate stops it the moment the outcome is decided, a `record_state` step captures the outcome (e.g. `match_outcome`) as run evidence, and `scenario sweep team_matchup_demo --seeds 1,2,3` surfaces each seed's outcome under `per_seed[].state_records` — a win-rate becomes a single read of `sweep.json`, with aggregation staying agent-side. See `examples/sample_project/qa/scenarios/examples/team_matchup_demo.json` for the full runnable document (two teams of two kinematic actors, rush vs hold, closed-loop `until` + `record_state`).

## Temporal assertions (S33/S52 additive contract)

Temporal assertions add bounded runtime windows on top of existing one-shot assertions. They are additive and do not change existing assertion semantics.

Supported temporal assertion names:

- `max_delta_position`
- `follows`
- `never`
- `within_limits`
- `observe_until`
- `trend`
- `frame_time`

Scenario examples:

```yaml
determinism:
  temporal_sampling:
    mode: fixed_delta
    fixed_delta_seconds: 0.0166667

assert:
  - max_delta_position:
      target: qa:demo_actor
      window_frames: 12
      max: 32
      sample_every_frames: 1
      include_context_before: 2
      include_context_after: 2
  - follows:
      follower: qa:companion
      leader: qa:demo_actor
      window_seconds: 1.0
      tolerance: 24
      sample_every_frames: 1
  - never:
      target: qa:warning_label
      text: value
      forbidden: "DESYNC"
      window_frames: 30
      sample_every_frames: 1
  - within_limits:
      target: qa:demo_actor
      window_frames: 20
      property: position_delta
      min: 0
      max: 48
      sample_every_frames: 1
  - observe_until:
      target: qa:main_menu
      source: qa:main_menu
      method: get_qa_state
      matches:
        status: pending
      window_frames: 8
      sample_every_frames: 1
      temporal_sampling:
        mode: max_frame_duration
        max_frame_duration_seconds: 0.0166667
  - observe_until:
      target: qa:spinner
      visible: true
      window_seconds: 0.25
      sample_every_frames: 1
  - observe_until:
      target: qa:status_label
      property: text
      operator: contains
      expected: Pending
      window_frames: 12
      sample_every_frames: 2
```

Bounded-window rules:

- Exactly one bounded window key is required: `window_frames` or `window_seconds`.
- `window_frames` must be a positive integer.
- `window_seconds` must be a positive number.
- Unbounded windows and non-positive windows fail validation as `scenario_validation_failed`.

Temporal sampling rules:

- `determinism.temporal_sampling` is optional and applies only while the scenario runner evaluates temporal assertions and temporal waits that support deterministic sampling.
- Assertion-local `temporal_sampling` overrides the scenario-level setting for that assertion only.
- Supported modes are `fixed_delta` with positive `fixed_delta_seconds`, and `max_frame_duration` with positive `max_frame_duration_seconds`.
- The duration field must match the selected mode; missing, mismatched, zero, negative, or unsupported modes fail scenario validation before runtime execution.
- Fixed-delta and max-frame-duration sampling are scoped to the QA scenario execution window and must not silently alter production timers outside that run.

`observe_until` syntax:

- `target` is required and resolves through the normal runtime locator rules.
- Exactly one condition family is required.
- Property conditions use `property`, `operator`, and optional `expected`; supported operators are `equals`, `contains`, `present`, `absent`, `approx_equals`, `approximately_equals`, `greater_than`, `less_than`, and `within`. `equals`, `contains`, `approx_equals`, `approximately_equals`, `greater_than`, `less_than`, and `within` require `expected`; `approximately_equals` is an alias for `approx_equals`, and both accept optional `epsilon` with default `0.0001`. `within` additionally requires a positive `tolerance` and matches when `|actual - expected| <= tolerance`.
- Visible conditions use `visible: true` or `visible: false` and inspect effective scene-tree visibility.
- QA method state conditions use `source`, `method`, and `matches`; `method` must identify a QA-prefixed/read-only state method returning JSON-compatible data, and `matches` maps dot-separated state paths such as `player.pending` or `status` to expected JSON-compatible values.
- Malformed state paths, unresolved player-visible or QA state targets, multiple condition families, missing windows, negative windows, zero `sample_every_frames`, and non-JSON-compatible method results fail validation or runtime with structured negative payloads. `observe_until` must not pass by reading hidden debug labels when the configured target/source cannot be resolved.

### State-expression observations (`within_limits`, `never`, `trend`)

`within_limits`, `never`, and `trend` can observe a state-provider expression instead of a node property, using the same `source` vocabulary as `state_matches` and `wait_until`. Declaring `source` switches the observation; omitting it keeps the legacy node/property behavior.

```yaml
assert:
  - within_limits:
      source: provider
      provider: match_state
      path: core.instability
      window_frames: 120
      min: 0
      max: 0.8
  - never:
      source: method
      target: qa:match_controller
      method: qa_instability_state
      path: phase
      forbidden: [meltdown]
      window_frames: 120
```

Rules:

- `source: provider` requires a registered state-provider id in `provider` (no node `target` is needed); `source: method` requires `target` plus a read-only QA `method`, exactly as in `state_matches`.
- `path` is an optional dot-separated path into the returned state; the value it selects must be numeric for `within_limits`/`trend` and is string-compared against `forbidden` for `never`. A missing path fails the assertion with `state_path_missing`; a non-numeric value fails with `state_value_not_numeric`.
- `never` cannot combine `source` with the legacy `property`/`text`/`token` observation fields.
- The state expression is re-read every sample inside the bounded window, so providers/methods must stay cheap and read-only; result payloads echo `source`, `provider`/`method`, and `path` alongside the usual window and sample evidence.

### `trend` (sampled numeric direction)

`trend` samples a numeric value over the bounded window and asserts its direction:

```yaml
assert:
  - trend:
      source: method
      target: qa:match_controller
      method: qa_instability_state
      path: value
      trend: increasing          # increasing | decreasing | non_increasing | non_decreasing
      min_delta: 0.2             # optional: |last - first| must reach this total change
      tolerance: 0.01            # optional: per-sample noise allowance for the strict variants
      window_frames: 120
      sample_every_frames: 2
```

- The observed value is a node observation (`target` plus optional `property`/`field`, defaulting to `position_delta` like `within_limits`) or a state expression (`source`/`provider`/`method`/`path` as above).
- `trend` is required and must be one of `increasing`, `decreasing`, `non_increasing`, `non_decreasing`.
- Strict variants (`increasing`/`decreasing`) require the correct total-delta sign and allow each consecutive sample to move against the trend by at most `tolerance` (default `0`). The `non_*` variants require every consecutive step to be `<= 0` / `>= 0` exactly and ignore `tolerance`.
- `min_delta` (optional, finite non-negative) additionally requires the total change in the trend direction to be at least `min_delta`.
- The window must produce at least two samples or the assertion fails with `invalid_window`.
- Result payloads echo `sample_count`, `first_value`, `last_value`, `total_delta`, and a bounded sample series (head and tail, capped at 32 entries with `omitted_samples` counting the rest); failures add `mismatch_type` (`sample_direction_mismatch`, `total_delta_sign_mismatch`, or `min_delta_not_met`) and the failing sample.

### `frame_time` (frame-rate assertion)

`frame_time` samples wall-clock frame durations over the bounded window and asserts aggregate frame-timing bounds inside the scenario:

```yaml
assert:
  - frame_time:
      window_frames: 300
      max_avg_ms: 16.7     # average frame time bound
      max_p95_ms: 20       # 95th-percentile frame time bound
      min_fps: 55          # worst-frame bound: 1000 / max frame ms must stay >= this
      sample_every_frames: 1
```

- At least one of `max_avg_ms`, `max_p95_ms`, `min_fps` is required; each must be a positive number.
- Aggregates are computed from per-sample wall-clock deltas: `avg_ms`, `p95_ms` (95th percentile), `max_ms`, and `min_fps` (`1000 / max_ms`, the worst instantaneous frame). With `sample_every_frames > 1` each sample averages the elapsed time across its cadence, which smooths `p95_ms`/`min_fps`; keep the cadence at `1` for honest percentile/worst-frame bounds.
- The result payload echoes `bounds`, the `aggregates`, and a bounded sample series (capped at 32 entries, `omitted_samples` counts the rest); failures list the violated bounds under `violations` and fail with `frame_time_mismatch`.
- **`frame_time` cannot be combined with `determinism.fixed_fps`.** Forced frame pacing makes wall-clock frame times meaningless (the run simulates a fixed delta while processing frames as fast as it can), so validation fails with `scenario_validation_failed`; drop `fixed_fps` or move the frame-rate gate into a separate perf scenario. `frame_time` also rejects assertion-local `temporal_sampling` and ignores scenario-level `determinism.temporal_sampling` for its own window, because retuning `Engine.max_fps`/physics ticks while measuring would corrupt the measurement.
- Under the CLI's default fastest frame pacing (vsync disabled, frames processed back-to-back) `frame_time` measures per-frame processing cost rather than presented FPS — that is the useful perf signal, but expect lower `avg_ms` than a realtime run; declare `time_control` when you need realtime pacing.

## Proximity assertions (S34 additive contract)

Saved scenarios can assert spatial proximity contracts with `distance` and temporal proximity changes with `distance_changed`.

Scenario examples:

```yaml
assert:
  - distance:
      a: qa:actor
      b: qa:target
      max: 96
  - distance:
      a: qa:actor
      b: qa:target
      min: 128
  - distance_changed:
      a: qa:actor
      b: qa:target
      min: 32
      window_frames: 12
      sample_every_frames: 1
```

Contracts:

- `distance.max` checks within-radius (`distance <= max`).
- `distance.min` checks outside-radius (`distance >= min`).
- `distance.min` and `distance.max` may be combined to check a bounded range.
- When both distance bounds are supplied, `min` must be less than or equal to `max`.
- `distance_changed.min` checks absolute distance change by at least the threshold across a bounded window.
- Use explicit stable locators such as `qa:actor`; assertions do not infer intent from node names.

Sampling rules:

- Sampling cadence is explicit via `sample_every_frames`.
- If omitted, default sampling cadence is `sample_every_frames: 1`.
- `sample_every_frames` must be a positive integer.

Supported observation sources for `never`:

- `text` reads the target's text property.
- `property` reads the named Godot property.
- `token` reads the named Godot property-like token used by runtime fixtures or project code.

Temporal validation/runtime failures are actionable and include command/assertion name, target(s), bounded window, expected bound, actual observed value, failing sample frame/time, and artifact references when available.

## Surface probe assertions (S36 additive contract)

Surface probe assertions add bounded project-owned surface classification checks. They are opt-in and require explicit QA provider configuration.

Supported surface assertion names:

- `surface_probe`
- `surface_matches`

Scenario examples:

```yaml
assert:
  - surface_probe:
      target: qa:demo_actor
      provider: qa:demo_actor
      provider_method: qa_probe_surface
      coordinate_space: world_2d
      surfaces: [walkable, blocked, water]
      points:
        - {x: 120, y: 420, expected: walkable}
        - {x: 184, y: 420, expected: blocked}
  - surface_matches:
      target: qa:demo_actor
      surface: walkable
      coordinate_space: world_2d
      provider: qa:demo_actor
      provider_method: qa_probe_surface
      actor_surface_method: qa_current_surface
```

Contracts:

- Providers are project-owned and explicit; no generic inference from physics/collision names is allowed.
- `surface_probe.target`, either `surface_probe.provider` or `surface_probe.provider_id`, and `surface_probe.points` are required by scenario validation.
- `surface_matches.target` and `surface_matches.surface` are required by scenario validation; the runtime samples the target's current point and compares it to the actor-reported state.
- `surface_matches` accepts optional bounded temporal fields: `window_frames` or `window_seconds`, optional `sample_every_frames`, and optional `dwell_frames` or `dwell_seconds`. Without these fields it remains a single immediate assertion.
- Temporal `surface_matches` passes only after the expected surface is observed for the requested dwell duration inside the bounded window; failures include sample history and provider/point evidence when available.
- With legacy target providers, `provider_method` must identify a QA-prefixed, read-only-by-convention method; when omitted at runtime, the target can provide `qa_surface_probe_method` metadata. With registered probe providers, use `provider_id` instead; the registered method receives a bounded request dictionary and returns a JSON-compatible dictionary.
- Actor surface state for `surface_matches` comes from `actor_surface_method` or target metadata `qa_surface_state_method`; the method must be QA-prefixed and return a JSON-compatible label.
- Provider responses must be JSON-compatible.
- Probe `points` use numeric `x`, `y`, and optional `z`; expected labels are attached per point with `expected`.
- `points` and `radius` are bounded; out-of-bounds point counts/radius fail validation or runtime with actionable failures.
- `surfaces` declares allowed labels for the assertion request; point `expected` labels outside that list fail scenario validation, and provider-returned unknown labels fail with `unknown_surface_label`.
- `coordinate_space` must be one of `world_2d`, `viewport_2d`, or `world_3d`; unsupported spaces fail with `unsupported_coordinate_space`.

Actionable failure classes include:

- `missing_probe_provider`
- `invalid_probe_result`
- `unknown_surface_label`
- `unsupported_coordinate_space`

- `probe_request_out_of_bounds`

Evidence requirements:

- Failures and mismatches should include probe evidence with point, coordinate space, expected label (when provided), actual label, and provider/provider-method metadata.

## Multi-viewport scenarios

Two or more `viewports` entries fan the run out into a viewport matrix — one run per viewport, aggregated under `data.viewports` + `data.runs[]`. Use `layout_audit` for broad coverage:

```yaml
name: sample_settings_layout
scene: res://ui/settings_menu.tscn
viewports:
  - [1280, 720]
  - [1920, 1080]
  - [2560, 1440]
  - [1280, 800]
locales: [en, es, pt_BR]
execution:
  isolation: cold
steps:
  - wait_frames: 5
  - snapshot: default
assert:
  - layout_audit: strict
  - no_errors: true
```

A viewport matrix is structurally cold (see Execution isolation above), so a
multi-viewport scenario that belongs to a `--warm --warm-strict` bundle should
declare `execution.isolation: cold`. Without it the strict structural check fails
the bundle with `multi_viewport forced this scenario to run cold in a --warm
--warm-strict bundle`; the checked-in `sample_settings_layout` in
`examples/sample_project` declares it for exactly that reason.

Each viewport executes the complete runtime scenario, including its final
`assert` entries and deterministic setup. CLI-side assertions such as
`layout_audit` remain CLI-side checks; they are not sent to the runtime addon.

The matrix is reserved for genuine multi-viewport fan-out (`viewports.len() > 1`). A scenario with **zero or one** `viewports` entry runs the normal single-run path — a single-viewport `layout_audit` produces the ordinary single-run envelope with layout failures attached inline, not a `runs[]` matrix envelope, and the lone viewport just sets the window resolution. This means a single-viewport `layout_audit` is warm-eligible (windowed-warm) rather than forced cold.

`viewports` must be a list of two-integer arrays `[width, height]`. Malformed entries are rejected as `scenario_validation_failed` instead of being silently coerced.
`screenshot_match` is not supported across a multi-viewport matrix (baseline naming/mapping needs a dedicated contract); the combination is rejected statically as `scenario_validation_failed`. `screenshot_match` with a single viewport — including alongside `layout_audit` — is fine.

## Scenario bundles

Scenario bundles are declarative manifests for feature-group regression gates.

Bundle manifest search path:
- `qa/gates/*.json|yaml|yml`

Required fields:
- `name`
- `required_scenarios`

Optional metadata:
- `purpose`
- `required_bundles`
- `optional_scenarios`
- `recommended_manual_checks`

Safe reference rules:
- Scenario references must resolve to declared scenario names from `scenario list`.
- Bundle references in `required_bundles` must resolve to declared bundle names from `scenario bundle list`.
- Path traversal, absolute-path references, missing included bundles, duplicate required scenarios, self-inclusion, and recursive bundle references are rejected with structured failures before child execution.
- Missing required scenarios fail the bundle; missing optional scenarios emit structured warnings and only fail under `--strict`.
- `scenario bundle run` defaults to CPU-aware process-level parallelism for runnable child scenarios. Omitted `--jobs` and `--jobs 0` use available parallelism with fallback `1`; `--jobs N` uses exactly `N` requested workers before runnable-count capping; `--jobs -N` reserves `N` available threads with lower bound `1`.

Behavior contract:
- Bundles are declarative data only.
- Bundles are not a CI scripting language.
- Unsupported fields such as `commands`, environment mutation, and shell execution fail validation.
- `scenario lint` remains separate from `scenario bundle run` unless a future explicit option is documented.
- Child scenario execution behavior remains sourced from direct `scenario run`.
- Child scenario `godot_args` and `project_args` remain scenario-owned and are preserved exactly as they are for direct `scenario run`, with `godot_args` before Godot's `--` separator and `project_args` after it.
- Bundle summaries include additive expansion metadata such as `source_bundle`, `bundle_chain`, `run_root`, `godot_args`, `project_args`, `launch_display`, and per-child status. `data.runs[]` remains ordered by original bundle expansion index even when children finish out of order. Persisted bundle metadata uses the same secret-looking `project_args` redaction contract as direct scenario artifacts.
- Bundle command JSON, `meta.json`, and `summary.json` include `parallelism` with `raw_jobs`, `resolved_jobs`, `source`, `available_parallelism`, `reserved_count`, `runnable_count`, and `complete_queued: true`.
- Bundle retry policy is CLI-owned, opt-in, bounded, and limited to infrastructure-only startup failures before assertions begin. Bundle manifests do not declare unbounded retry loops or override child scenario behavior.
- Negative retry cases remain strict failures with no retry: gameplay/assertion failures, script errors, malformed scenario definitions, malformed or invalid `result.json`, screenshot quality failures, visual assertion failures, log health failures, crashes after scenario execution begins, and normal process failures.
- Child scenario behavior remains sourced from direct `scenario run`; bundle retry metadata only wraps attempts and must not change the assertion, screenshot, log, or artifact contract of the child scenario.

Example composed bundle:

```yaml
name: composed_regression_gate
purpose: Regression gate that composes reusable menu coverage with direct checks.
required_bundles:
  - reusable_menu_gate
required_scenarios:
  - sample_settings_layout
  - bundle_godot_args_demo
```

Example screenshot-heavy bundle:

```yaml
name: screenshot_heavy_regression_gate
purpose: Regression gate for screenshot-backed visual scenarios that require real rendered pixels.
required_scenarios:
  - visual_occlusion_demo
  - saved_snapshot_pixels_demo
  - visual_metadata_rect_demo
  - visual_line_indicator_demo
```

`recursive_bundle_demo` in the sample project is an intentional negative fixture. Running it should fail with `bundle_cycle`; it is present to document recursive-bundle failure wording, not as a happy-path gate.

## Live gate manifests

Live gates are declarative manifests for hidden-overlay live inspection profiles. They are loaded from `qa/live_gates/*.json|yaml|yml` and executed with `godot-qa live-gate run <name> --project <project> --json`.

Required fields:

- `name`: unique safe gate name.
- `scene`: scene path used when the CLI starts a live session instead of attaching to one.
- `anchors`: non-empty list of anchor declarations.

Anchor fields:

- `name`: unique safe anchor name within the gate.
- `locator`: durable runtime locator such as `qa:main_menu`, `path:/root/...`, or `name:StartButton`; prefer `qa:` anchors for long-lived contracts.
- `required`: optional boolean, default `true`. Required anchors fail the gate when missing. Optional anchors emit structured warnings unless `live-gate run --strict` is used.
- `methods`: optional list of read-only QA state method checks. Each entry declares `method`, optional `source: "method"`, and optional `matches` using the same state-match operators as `assert.state_matches`.
- `probes`: optional list of surface probe checks. Each entry declares `provider`, `provider_method`, `coordinate_space`, `surfaces`, and bounded `points` using the same contracts as `assert.surface_probe`.

Surface probe assertions remain point assertions inside scenarios and live gates. `surface audit` reuses the same explicit project-provider boundary for broader spawn, corridor, and mirrored-region checks, but it does not change existing `assert.surface_probe`, `assert.surface_matches`, or `probes` semantics. Scenario assertions still evaluate only their declared expected surfaces and points; they do not inherit audit metrics such as `spawn_clearance_min_distance`, `minimum_corridor_width`, or `symmetry_delta_score`.

Optional top-level fields:

- `description`: human-readable summary returned by `live-gate list`.
- `strict`: default strictness for optional-anchor warnings; the CLI `--strict` flag can only make validation stricter.
- `session`: live-session defaults such as `viewport`, `headless`, `windowed`, and `hide_overlay`.
- `overlay`: project-owned overlay verification hook, usually a durable anchor plus read-only QA state method/probe that reports whether debug overlays are hidden.
- `artifacts`: diagnostic artifact preferences. Live gate artifacts are evidence only and must not approve or update files under `qa/baselines`.

Example:

```yaml
name: hidden_overlay_inspection
scene: res://ui/main_menu.tscn
description: Verify hidden-overlay runtime anchors
session:
  hide_overlay: true
overlay:
  anchor: qa:debug_overlay
  method: get_qa_overlay_state
  matches:
    - {path: visible, operator: equals, expected: false}
anchors:
  - name: main_menu
    locator: qa:main_menu
    required: true
    methods:
      - method: get_qa_menu_state
        matches:
          - {path: screen, operator: equals, expected: main}
  - name: optional_perf_panel
    locator: qa:perf_panel
    required: false
    methods:
      - method: get_qa_panel_state
```

Manifest behavior:

- Live gate manifests are declarative data, not scripts.
- Anchors must be durable locators. Missing optional anchors produce warnings by default; missing required anchors always fail.
- `methods` and `probes` are allowlisted read-only observation surfaces. Rejected method names, non-JSON-compatible return values, probe mismatches, unknown surface labels, and unsupported coordinate spaces fail with structured diagnostics.
- Strict mode promotes optional-anchor warnings and manifest warnings to failures.
- Hidden-overlay checks must use documented project-owned QA surfaces. A visually hidden overlay is not enough if configured durable anchors cannot be inspected, and projects without an overlay hook should warn or fail clearly instead of pretending hidden-overlay evidence exists.

## Asset trial validation scenarios

An asset-trial validation uses one ordinary scenario with deterministic
`seed`, `fixed_fps`, and `locale` settings, plus existing gameplay assertions.
Each selected texture snapshot or audio capture must have a `record_state` step
immediately before and after it. The checkpoints use the same bounded,
read-only state sources as all other `record_state` steps. Texture evidence must
enable saved screenshots; trials reject `screenshot_match` assertions on the
selected texture snapshots because candidate artwork is expected to differ.
Audio evidence must capture the selected player's bus while the trial WAV is
playing; short sounds that cannot be observed during the synchronous capture do
not count as verified playback.

The sample scenario `asset_coin_pickup` starts a gameplay scene that instances
the separately edited coin scene. It captures a visible coin, schedules the
pickup before a synchronous SFX capture, and checks that gameplay collected one
coin. Its sound player remains alive after the pickup sprite is removed. See
`examples/sample_project/qa/scenarios/examples/asset_coin_pickup.yaml` and the
paired trial manifest at `examples/sample_project/qa/asset-trials/coin.yaml`.
