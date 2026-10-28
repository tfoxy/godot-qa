# Test Strategy

## Implementation-platform verification note

The shipped verification baseline is Rust-only. `cargo test`, host-binary smoke commands, and release-artifact validation are the default repository gates.

## Verification layers

The verification posture has three layers:

### 1. Contract guardrails

- `crates/godot-qa-cli/tests/repo_contract.rs` pins parser/public-surface inventory, JSON envelope and failure-field stability, and cross-references between `docs/cli-spec.md`, `docs/scenario-format.md`, `docs/runtime-bridge-protocol.md`, `docs/artifact-format.md`, and the shipped README/example docs.
- These tests fail loudly when documented behavior and implemented behavior drift.

### 2. Rust unit and integration coverage

- `cargo test` across the workspace for crate-local behavior (parsing, protocol codecs, assertion evaluation, artifact writing).
- process/runtime/scenario integration tests against controlled fixtures (fake game sockets, fixture projects).
- `artifact_export_cli` verifies exact PNG bytes and hashes, distinct hashes for projected JSON, portable relative links after relocation, sanitization of source result details, explicit hash-bound reviews, every source failure's retention, no overwrite, missing results, and symlink escape rejection.

### 3. Godot-backed smoke coverage

- sample-project commands run through the Rust binary against `examples/sample_project`.
- runtime protocol smoke against the real Godot addon (`addons/godot_qa`) without changing the bridge contract.

## Layers

### Unit tests

Run without Godot.

- CLI argument parsing.
- JSON result format.
- Scenario parsing.
- Artifact path creation.
- `scenario run` result mapping for launch, artifact, and invalid-runtime-payload failures.
- `import` and `check` result shaping, scene/script discovery exclusions, manifest validation, and aggregate failure flattening.
- Screenshot diff math where possible.
- Layout audit logic using fixture UI JSON.
- MCP adapter argv mapping and stdio server smoke coverage without requiring Godot.

### Integration tests with Godot

Run when Godot is available.

- `doctor` detects Godot.
- disposable test project copies in `addons/godot_qa`.
- sample scene opens through `qa_runner.gd`.
- `import` succeeds against the sample project and writes stable import artifacts.
- bounded `check --scene` passes healthy scenes, fails the runtime-error fixture, and keeps layout-only failures out of scope.
- bounded `check --all-scenes` runs import once, enumerates real project scenes with explicit exclusions, and flattens per-scene failures.
- bounded `check --script` validates GDScript load/compile health without instantiating the script or requiring `run_qa_test()`.
- bounded `check --all-scripts` enumerates project scripts with explicit addon/generated/artifact exclusions and flattens per-script failures.
- opt-in `check --tests` runs one manifest-backed scene test plus one manifest-backed script test from the sample project.
- deliberate slow-summary fixtures lower `slow_summary_threshold_ms` so validation can assert artifact-backed slow evidence without making the run fail.
- one-shot scenario runs create `meta.json`, `request.json`, `result.json`, and `logs.txt`.
- `snapshot` steps create `screenshot.png`, `tree.json`, `ui.json`, `focus.json`, and `errors.json`, and runtime failures after a snapshot keep those paths attached in `failure.artifacts` while the CLI also preserves the run-level `run_root`, `result_path`, `logs_path`, `screenshot_paths`, and `diagnostic_paths`.
- direct bridge coverage should verify `tree.dump`, filtered `tree.dump` params (`target`, `max_depth`, `qa_only`, `include_metadata`, `metadata_keys`), `ui.dump`, `focus.get`, `focus.graph`, `node.inspect`, `errors.read`, one active input command path, and structured `unsupported_command` handling while QA mode is enabled.
- runtime snapshot smoke coverage should verify filtered `snapshot.capture` writes the preserved `screenshot.png`, `tree.json`, `ui.json`, `focus.json`, and `errors.json` filenames, records filter metadata in JSON artifacts, does not filter screenshots, and fails invalid filter params or unresolved targets with actionable structured errors.
- live screenshot coverage should verify `session start` is windowed by default, `screenshot capture` stages a session PNG plus copies `--out`, `screenshot compare --live` reuses the normal screenshot-compare engine against a checked-in baseline when image support is available, and headless sessions reject screenshot-producing commands.
- Live window-present regression: in an isolated headless Gamescope compositor, start a static scene in a project with `application/run/low_processor_mode=true` and capture the compositor's window pixels before sending any input. The scene must replace the Godot launch splash without needing a click or key press. A QA viewport screenshot alone cannot prove this: it can contain the scene even when the first frame was drawn without swapping the window buffers.
- one-shot scenarios should prove action input, target clicks, held-input persistence until release, and path/name/qa locator diagnostics against real runtime state changes.
- recorder coverage should prove `scenario record` creates `.qa/artifacts/recordings/<name>/<run-id>/`, exports a raw trace through `record.export`, writes a replayable candidate under `qa/scenarios/candidates/`, and preserves review-only `suggested_assertions`. Gamepad capture is part of that contract: recorded `input_joy_button`/`input_joy_motion` events must convert into validating `input_joy_button`/`input_joy_motion` steps with interleaved `wait_frames` timeline gaps, honoring the tap/hold pairing and the quantization thresholds (`--joy-motion-step`/`--joy-motion-deadzone`/`--joy-motion-frame-gap`).
- headless screenshot coverage should verify non-visual commands still work in headless sessions, screenshot-producing live commands fail with `headless_screenshot_unsupported`, and screenshot-backed scenario runs either select a real display backend or safely detected virtual display wrapper, or fail before runtime execution with `display_backend_unavailable`.
- screenshot diff coverage should prove exact match, mismatch evidence (`diff.png` plus metrics), missing baseline failures, and that `scenario run` strips `screenshot_match` before Godot launch while preserving `request.json`.

### Validation surface guardrails

Keep the post-S21 validation surface covered in two layers:

- CLI-only guardrails:
  - parser visibility for `doctor`, `import`, and `check`
  - structured parse failures for invalid validation invocations
  - dispatch proof that `import` / `check` do not regress to generic `not_implemented`
  - import/check result-shaping tests with fake Godot launches for launch failures, non-zero exits, invalid runtime payloads, and aggregate failure flattening
- Godot-backed smoke coverage:
  - `import` against `examples/sample_project`
  - `check --scene res://ui/main_menu.tscn` as the expected-pass bounded validation case
  - `check --scene res://ui/fixtures/runtime_error_probe.tscn` as the expected-failure actionable-runtime-error case
  - `check --all-scenes` as the expected-failure aggregate case
  - `check --script res://ui/main_menu.gd` as the expected-pass bounded GDScript load/compile case
  - `check --all-scripts` as the expected-pass project script enumeration case
  - `check --tests` as the expected-pass manifest-backed scene/script smoke case
  - `check --scene res://ui/fixtures/bad_settings_menu.tscn` as an explicit non-goal pass case showing layout failures remain out of scope for bounded `check`

S60 check-test selection guardrails:

- Keep filtered `check --tests` coverage alongside the unfiltered full-manifest regression command:

  ```bash
  cargo run --bin godot-qa -- check --tests --test sample_script_test --project examples/sample_project --json
  cargo run --bin godot-qa -- check --tests --tag smoke --project examples/sample_project --json
  cargo run --bin godot-qa -- check --tests --profile smoke --project examples/sample_project --json
  cargo run --bin godot-qa -- check --tests --project examples/sample_project --json
  ```

- Assert JSON selection metadata for `total_test_count`, `selected_test_count`, `skipped_test_count`, and `selection` whenever filter behavior changes.
- Filtered runs must not mask selected-test failures. Any failing selected scene or script test still fails the command and contributes normal per-test failure details.
- Skipped tests are accounting metadata only. They must not appear as passed tests, and they must not be included in selected-test pass counts.
- Unknown `--test`, `--tag`, or `--profile` values, malformed profiles, duplicate tags, reserved tags, ambiguous filter combinations, and filters that select no tests should fail before Godot launch with actionable validation failures.

S61 check-test batch guardrails:

- Batch validation supplements the isolated `check --tests` regression coverage; it does not replace the unbatched commands above because `--batch` intentionally trades process isolation for speed.
- Keep an explicit batch smoke command and scenario regression command in validation when batch behavior changes:

  ```bash
  cargo test repo_contract_check_tests_batch_docs
  cargo run --bin godot-qa -- check --tests --batch --tag smoke --project examples/sample_project --json
  cargo run --bin godot-qa -- scenario run sample_main_menu_start --project examples/sample_project --json
  ```

- Assert `execution_mode`, the batch summary artifact path, and normal per-test artifact directories in JSON and artifact inspections.
- Selected tests with `batch=false` must be rejected in batch mode before launch; they must not be silently skipped or run in isolated mode.
- Ordinary selected-test failures should fail the aggregate while preserving later selected results whenever the Godot process can continue.
- Contamination diagnostics should be checked when practical for held input, scene/script instances, `Engine` settings, and runtime errors between entries.
- Batch mode snapshots `Engine.time_scale`, `Engine.physics_ticks_per_second`, `Engine.max_fps`, and `Engine.max_physics_steps_per_frame` right after applying a test's own declared `time_control` (its expected state) and compares them again right after that test finishes; a mismatch means the test's own runtime code mutated those settings and did not restore them, and is reported as `runtime_contamination_detected` scoped to that test's id even though the batch runner always restores the project baseline afterward regardless.

S62 progress, timeout, and slow-summary guardrails:

- Keep the S62 public contract visible with `cargo test repo_contract_progress_timeouts_slow_summaries_docs`.
- Required validation commands for this contract:

  ```bash
  cargo test repo_contract_progress_timeouts_slow_summaries_docs
  cargo run --bin godot-qa -- check --tests --tag smoke --project examples/sample_project --json
  cargo run --bin godot-qa -- scenario run sample_setup_steps --project examples/sample_project --json
  ```

- Full runtime validation for the S62 implementation should additionally exercise the deliberate slow fixtures:

  ```bash
  cargo run --bin godot-qa -- check --tests --test slow_manifest_test --timeout-seconds 5 --project examples/sample_project --json
  cargo run --bin godot-qa -- scenario run slow_step_progress_demo --project examples/sample_project --json
  ```

- Slow fixtures such as `slow_manifest_test` and `slow_step_progress_demo` lower `slow_summary_threshold_ms` so `slow-summary.json` can be asserted without making successful runs fail.
- Hung fixtures should fail with structured timeout types such as `check_test_timeout` or `scenario_run_timeout`, preserve partial artifacts, and include enough progress evidence to identify the active test or step without process-list inspection.
- Manual timeout artifact inspection should open the command JSON, `progress.jsonl`, `slow-summary.json` when present, `logs.txt`, and any active test or scenario `result.json`. The artifact evidence should show active id/index, elapsed time, configured timeout, expected state, actual state, artifact paths, and owned-process cleanup evidence.
- Progress and timeout artifacts must preserve redaction expectations for session tokens and secret-looking project args.

S63 check-test profiling guardrails:

- Keep profiled `check --tests` coverage alongside the S60 selection and S62 progress regressions:

  ```bash
  cargo run --bin godot-qa -- check --tests --profile --test sample_script_test --project examples/sample_project --json
  cargo run --bin godot-qa -- check --tests --profile --tag smoke --project examples/sample_project --json
  cargo run --bin godot-qa -- scenario profile sample_main_menu_start --project examples/sample_project --json
  cargo run --bin godot-qa -- check --tests --tag smoke --project examples/sample_project --json
  ```

- Assert `data.profile`, `data.artifacts.profile`, bounded `per_test_timings`, selected-test outcome summaries, and normal per-test failures when profile behavior changes.
- Manual artifact inspection should open the referenced `.qa/artifacts/check/<run-id>/profile.json` and confirm `frame_times_ms` is bounded by `sample_limits.max_frame_samples`, `frame_time_percentiles_ms` is present, monitor counter summaries are present when Godot exposes samples, optional unsupported monitors are diagnostic only, and `check_test_total` timing rows identify where selected manifest-test time was spent.
- Normal unprofiled `check --tests` output must remain compatible and must not include large profile sample history.

### End-to-end scenarios

Run on real user projects or sample projects.

- main menu opens.
- pause menu opens on action.
- settings navigation works.
- layouts pass at common resolutions.
- important HUD overlays can be checked for containment and non-overlap even when they are not interactive controls.
- shared-camera gameplay scenes can prove that critical world actors stay inside the active camera framing.

## CI and local verification

GitHub Actions checks both supported desktop platforms. Local verification has two layers:

```text
CLI-only gate    Always available, no Godot required.
Godot gate       Required for runtime changes and release acceptance.
```

For local verification, use:

- `cargo test`
- `cargo build --release --bin godot-qa`
- `target/release/godot-qa doctor --project examples/sample_project --json`
- `target/release/godot-qa scenario list --project examples/sample_project --json`

The repository CI runs native Linux and Windows checks and Godot 4.5.2 runtime smoke coverage. See `.github/workflows/` for the executable configuration and [CONTRIBUTING.md](../CONTRIBUTING.md) for local commands. Run `cargo xtask prepare-sample` before import or smoke checks. Visual tests require a display backend; use Xvfb on Linux CI and the native display on Windows. Specialized Gamescope presentation acceptance remains an explicit Linux-only gate.

The bundled YAML examples run through the built-in scenario subset loader. Broader YAML syntax remains an internal-development concern rather than a supported end-user install mode.

## Agent verification

Every implementation spec should include the best available verification command in its completion response.

Examples:

```bash
cargo test
godot-qa doctor --project examples/sample_project --json
godot-qa scenario list --project examples/sample_project --json
godot-qa scenario run sample_main_menu_start --project examples/sample_project --json
godot-qa scenario run sample_settings_layout --project examples/sample_project --json
godot-qa scenario run sample_world_inside_viewport --project examples/sample_project --json
godot-qa layout audit --project examples/sample_project --scene res://ui/settings_menu.tscn --json
godot-qa node inspect qa:start_button --project examples/sample_project --json
godot-qa physics inspect qa:physics_probe --project examples/sample_project --radius 300 --frames 2 --json
godot-qa physics overlay qa:overlay_actor --project examples/sample_project --radius 400 --frames 2 --out .qa/artifacts/manual/physics_overlay_actor.png --json
godot-qa physics overlay qa:oversized_collider_actor --project examples/sample_project --radius 400 --frames 2 --out .qa/artifacts/manual/physics_overlay_oversized.png --json
godot-qa physics overlay qa:overlay_actor --project examples/sample_project --scenario physics_overlay_demo --snapshot aligned --out .qa/artifacts/manual/physics_overlay_from_snapshot.png --json
godot-qa trace record --project examples/sample_project --target qa:trace_probe --frames 60 --sample-every-frames 1 --physics --json
godot-qa focus graph --project examples/sample_project --json
godot-qa errors --project examples/sample_project --json
godot-qa check --project examples/sample_project --tests --json
```

S67 physics overlay smoke guidance:

- Discovery: `cargo run --bin godot-qa -- scenario list --project examples/sample_project --json` should include `physics_overlay_demo`.
- Aligned overlay: run live `physics overlay qa:overlay_actor` and verify JSON reports `.base.png`, overlay PNG, `.overlay.json`, nonzero shape/body counts, and no visual-footprint mismatch warnings.
- Oversized warning: run live `physics overlay qa:oversized_collider_actor` and verify the warning list includes `physics_overlay.collider_extends_far_beyond_visual_footprint` unless `--no-project-state` intentionally disables comparison.
- Snapshot review: run the sample `physics_overlay_demo` scenario, then render `physics overlay --scenario physics_overlay_demo --snapshot <saved-snapshot>` from saved evidence. The review path must consume `result.json.physics_overlays` and the saved `snapshots/<snapshot>/screenshot.png`; it must not recapture live state.
- Regression guard: keep `godot-qa physics inspect qa:overlay_actor --project examples/sample_project --radius 400 --frames 2 --json` passing so overlay collection does not break the existing inspect contract.
- Artifact review: generated overlay images and metadata are evidence only. They do not update or approve screenshot baselines, and test checks should assert warnings and artifact paths rather than aesthetic image quality.

S68 collision-report smoke guidance:

- Public contract guard: keep `cargo test -p godot-qa-cli repo_contract_collision_report_docs` passing so command syntax, threshold flags, provider names, warning IDs, and artifact paths stay visible in docs.
- Live smoke: run `cargo run --bin godot-qa -- physics collision-report qa:slide_actor --project examples/sample_project --frames 180 --sample-every-frames 1 --physics --json` and verify `data.artifacts.report`, linked trace artifact paths, metric keys, and warning objects.
- Strict warning smoke: run `cargo run --bin godot-qa -- physics collision-report qa:bouncy_actor --project examples/sample_project --frames 180 --sample-every-frames 1 --physics --max-normal-retention 0.35 --strict-warnings --json` and verify the expected failure is reported only when `collision_report.normal_velocity_retention_high` or related warning evidence is present.
- Trace regression: run `cargo run --bin godot-qa -- trace record --project examples/sample_project --target qa:slide_actor --frames 60 --sample-every-frames 1 --physics --json`, then `cargo run --bin godot-qa -- physics collision-report --from-trace .qa/artifacts/latest/trace.json --json` and verify the report summarizes the trace without rerunning Godot.
- Required regressions: keep `cargo run --bin godot-qa -- physics inspect qa:slide_actor --project examples/sample_project --radius 200 --frames 2 --json` and `cargo run --bin godot-qa -- scenario run sample_main_menu_start --project examples/sample_project --json` passing when collision-report behavior changes.
- Human review: collision-report evidence complements human playability review. Tests should assert metrics, warnings, provider handling, and artifact paths, not that high bounce is always a failure.

S69 surface audit validation:

- Public contract guard: keep `cargo test -p godot-qa-cli repo_contract_surface_audit_docs` passing so command syntax, metric keys, warning IDs, approved provider method names, artifact paths, heatmap behavior, and fixture anchors stay visible in docs.
- Focused unit tests: keep the `surface_audit` parser, metric, warning, artifact path, strict-warning, and heatmap renderer tests passing for bounded region, route, and mirror modes.
- Regression commands: run `cargo run --bin godot-qa -- scenario run surface_probe_demo --project examples/sample_project --json` and `cargo run --bin godot-qa -- assert surface-matches qa:demo_actor --project examples/sample_project --snapshot after_move --json` when surface audit behavior changes, because existing scenario assertions must remain unchanged.
- Sample fixture anchors: `res://ui/fixtures/surface_audit_demo.tscn` exposes `qa:surface_provider`, `qa:clear_spawn`, `qa:blocked_spawn`, `qa:route_start`, `qa:route_mid`, `qa:route_finish`, `qa:left_lane`, `qa:right_lane`, `qa:centerline`, plus named cases for clear spawn, blocked spawn, narrow corridor, dead-end pocket, soft blocker, hard blocker, and mirrored-delta layout.
- Artifact review: generated `surface_audit.json` reports and `.png` heatmaps are evidence only. They do not approve baselines or mutate runtime overlays.

S71 event trace validation guidance:

- Public contract guard: keep `cargo test -p godot-qa-cli repo_contract_event_trace_docs` passing so command names, provider method names, artifact paths, summary fields, warning IDs, no-debug-label guidance, and trace-record integration status stay visible in docs.
- Supported path: `trace record --events-provider` is not implemented in the current public command surface; `event trace` is the supported event stream path for VFX/SFX/UI/collision correlation, and existing `trace record` behavior must remain unchanged.
- Provider contract: approved read-only provider method names are `get_qa_event_stream_state`, `get_event_stream_qa_state`, `get_qa_events`, and `qa_event_stream`; tests must not infer events from node names, logs, screenshots, debug labels, engine audio internals, or engine VFX internals.
- Sample fixture smoke: run `cargo run --bin godot-qa -- event trace --project examples/sample_project --target qa:event_source --provider get_qa_event_stream_state --frames 120 --sample-every-frames 1 --json` and verify `.qa/artifacts/live/<session-id>/events/<event-trace-id>/events.json`, `summary.json`, required summary fields, and warning objects.
- Summarizer smoke: run `cargo run --bin godot-qa -- event summarize .qa/artifacts/live/latest/events/latest/events.json --json` after a live event trace and verify the summary is produced without rerunning Godot.
- Strict warning smoke: run `cargo run --bin godot-qa -- event trace --project examples/sample_project --target qa:event_source --provider get_qa_event_stream_state --frames 120 --pairing-config qa/event_pairing_demo.json --strict-warnings --json` through a validator wrapper when the fixture intentionally demonstrates `event_trace.event_spam_risk` or `event_trace.paired_event_missing`.
- Human review: event traces provide explicit project-owned evidence for correlation. They do not auto-approve screenshot baselines, do not add visible debug labels, and do not make project-specific event names part of the generic contract.

Artifact summary validation guidance:

- Review workflow: agents should run `godot-qa artifact summarize <run-root> --json` before manually opening screenshots, logs, traces, audio, physics, visual audit, bundle, live-session, or manual evidence folders. Use the summary as the review index, then open only the referenced evidence needed to explain warnings or failures.
- Public contract guard: keep `cargo test -p godot-qa-cli s76_artifact_summary_docs_contract` passing so command syntax, `artifact_summary.json`, category arrays, warning rule IDs, Markdown headings, and review workflow wording stay visible in docs.
- Focused checks: keep artifact-summary unit coverage for category classification, `--type`, `--warnings-only`, `--expect`, missing-root failure, default `artifact_summary.json` writing, `--no-write`, Markdown output, and no binary duplication behavior.
- Scenario validation: run `cargo run --bin godot-qa -- scenario run sample_main_menu_start --project examples/sample_project --json`, then `cargo run --bin godot-qa -- artifact summarize .qa/artifacts/sample_main_menu_start/latest --json`.
- Bundle validation: run `cargo run --bin godot-qa -- scenario bundle run sample_regression_gate --project examples/sample_project --json`, then `cargo run --bin godot-qa -- artifact summarize .qa/artifacts/bundles/sample_regression_gate/latest --expect scenario_results --json`.

Later specs add stronger runtime verification such as:

```bash
godot-qa layout audit --project examples/sample_project --scene res://ui/fixtures/bad_settings_menu.tscn --json || true
godot-qa screenshot compare --actual .qa/artifacts/.../screenshot.png --baseline qa/baselines/... --json
godot-qa session start --project examples/sample_project --viewport 1280x720 --json
godot-qa screenshot capture --project examples/sample_project --out .qa/artifacts/manual/main_menu_720p_candidate.png --json
godot-qa screenshot compare --project examples/sample_project --live --baseline .qa/artifacts/manual/main_menu_720p_candidate.png --json
godot-qa screenshot sequence --project examples/sample_project --scenario screenshot_sequence_demo --frames 120 --every 20 --out .qa/artifacts/manual/sequence_demo --json
godot-qa screenshot contact-sheet .qa/artifacts/manual/sequence_demo --columns 3 --thumb-width 320 --out .qa/artifacts/manual/sequence_demo_contact_sheet.png --json
godot-qa visual audit --screenshot .qa/artifacts/manual/current.png --targets .qa/artifacts/manual/current.targets.json --json
godot-qa visual audit --project examples/sample_project --live --target qa:readable_target --annotate-out .qa/artifacts/manual/visual_audit.png --json
godot-qa session stop --project examples/sample_project --json
godot-qa scenario record main_menu_recording --scene res://ui/main_menu.tscn --json
godot-qa assert in_camera_view qa:support_helper --project examples/sample_project --json
```

Screenshot sequence/contact-sheet validation (S72):

- Unit/doc contract: `cargo test -p godot-qa-cli repo_contract_screenshot_sequence_docs` verifies command syntax, manifest fields, contact-sheet companion JSON, per-frame quality, visual-risk warnings, manual baseline approval language, and required validation commands.
- Scenario listing smoke: `cargo run --bin godot-qa -- scenario list --project examples/sample_project --json` must include `screenshot_sequence_demo` so optional `--scenario` name resolution has a fixture without requiring a live Godot run.
- Runtime smoke when Godot/display is available: start `res://ui/fixtures/screenshot_sequence_demo.tscn`, run `screenshot sequence --frames 120 --every 20`, build `screenshot contact-sheet --columns 3 --thumb-width 320`, and stop the session.
- Failure behavior: missing or ambiguous `--scenario` names fail before capture with actionable JSON. Sequence capture does not replay scenario setup steps or assertions, and low-information frame failures remain `screenshot_low_information` unless `--allow-low-information` is supplied.
- Review boundary: per-frame visual-risk warnings are generic artifact-backed risk signals. Contact sheets, sequence manifests, frame PNGs, and diffs are review evidence only; baseline approval remains manual.

Visual validation guidance (S73):

- Public contract guard: keep `cargo test -p godot-qa-cli docs_visual_audit_contract` passing so visual audit source modes, threshold flags, warning IDs, metrics, annotation artifact language, and manual-review boundaries stay visible in docs.
- Source-mode coverage should exercise saved screenshot mode with `--screenshot` plus `--targets`, live mode with `--live --target`, sequence mode with `--sequence`, and scenario artifact mode with `--scenario` plus saved snapshot evidence when available.
- Strict warning coverage should verify that `--strict-warnings` converts findings such as `visual_audit.target_too_small`, `visual_audit.low_luminance_contrast`, or `visual_audit.target_offscreen` from warning-only evidence into a non-zero command result.
- Artifact checks should assert machine-readable metrics, effective thresholds, warning IDs, `frames_checked` sequence aggregates, and the optional `annotated` PNG path from `--annotate-out`; tests should not assert subjective image beauty or style quality.
- Review boundary: visual audit is a non-aesthetic, measurable review aid. It must not auto-approve baselines, must not replace screenshot compare or visible-pixel assertions, and must not infer target bounds from arbitrary colors when explicit target data is unavailable.

Review template validation (S75):

- Unit/doc contract: `cargo test -p godot-qa-cli repo_contract_review_templates_docs` verifies command docs, template IDs, JSON fields, generated run-command guidance, project-agnostic constraints, optional `review run` wording, and manual-review boundary language.
- Template inventory command: `cargo run --bin godot-qa -- review list-templates --json` must list `scenario-smoke`, `live-visual-review`, `live-physics-review`, `live-audio-review`, `live-event-review`, `visual-physics-audio`, and `deterministic-runtime-review` with descriptions, output types, required parameters, optional parameters, required tools, and generated command types.
- Sample-project generated review checks should initialize a scenario harness with `cargo run --bin godot-qa -- review init scenario-smoke --project examples/sample_project --name smoke_review --scene res://ui/main_menu.tscn --out .qa/review-scenarios/smoke_review.yaml --json`, then run it with `cargo run --bin godot-qa -- scenario run --file .qa/review-scenarios/smoke_review.yaml --project examples/sample_project --json`.
- Live-script generated review checks should initialize deterministic runtime evidence with `cargo run --bin godot-qa -- review init deterministic-runtime-review --project examples/sample_project --name deterministic_review --scene res://ui/main_menu.tscn --project-arg --seed --project-arg 123456 --frames 60 --out .qa/review-scripts/deterministic_review.yaml --json`, then run it with `cargo run --bin godot-qa -- session run-script .qa/review-scripts/deterministic_review.yaml --project examples/sample_project --json`.
- Regression coverage must preserve existing `scenario init`, `scenario record`, `scenario run --file`, `session run-script`, launch-argument forwarding, and artifact layout behavior. Project arguments are forwarded exactly as strings and are never interpreted by review templates.
- Review boundary: templates accelerate exploratory QA only. Generated artifacts are normal scenario/live evidence; they do not auto-approve baselines, do not write `qa/baselines`, and do not replace final human review.

Runtime QA performance guidance (S40):

- Required repo commands for normal S40 verification:
  - `cargo test`
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo run --bin godot-qa -- scenario list --project examples/sample_project --json`
- Use durable `qa:` anchors for stable targets, project-owned `get_qa_*_state()` methods for aggregate dynamic gameplay state, named `qa_*_probe()` methods for bounded spatial/collision/surface facts, and screenshot-backed assertions for rendered perception.
- Treat debug labels as diagnostics unless their text is the user-facing contract under test.
- External projects may provide optional downstream stress validation and motivation for filtered inspection behavior. They are advisory only, not required fixtures and not CI gates for `godot-qa`.

QA state method assertion allowlist guidance (S48):

- `assert.state_matches` method sources are limited to zero-argument, read-only-by-convention, JSON-compatible QA state methods named `qa_*`, `get_qa_*`, or `get_*_qa_state`; they are not an arbitrary method-call escape hatch.
- Prefer durable `qa:` anchors plus `get_*_qa_state()` or `get_qa_*_state()` aggregate state methods for gameplay summaries. Use setup `call_method` for deterministic fixture mutation, named `qa_*_probe()` methods for bounded spatial/surface checks, screenshot-backed assertions for rendered perception, and debug-label assertions only when the label text is the user-facing contract.
- Required validation commands:
  - `cargo test state_matches_docs_contract`
  - `cargo test state_matches_rejected_method_contract`
  - `cargo test state_matches_numeric_equals_mismatch_reports_precision`
  - `cargo run --bin godot-qa -- scenario run state_matches_get_named_qa_state_demo --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario run state_matches_array_method_demo --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario run sample_setup_steps --project examples/sample_project --json`

QA state schema validation guidance (S74):

- `qa-state validate` covers live QA method returns, saved JSON input, and trace-selected state artifacts. It complements `state_matches`; it does not replace scenario assertions and does not require every project or QA method to publish a schema.
- Focused docs/CLI contract validation:
  - `cargo test -p godot-qa-cli repo_contract_qa_state_schema_docs`
  - `cargo test -p godot-qa-cli qa_state_validate`
- Sample fixtures and schema:
  - `examples/sample_project/ui/fixtures/qa_state_schema_demo.tscn`
  - `examples/sample_project/qa/schemas/schema_demo_state.schema.json`
  - `examples/sample_project/qa/fixtures/schema_demo_state_valid.json`
  - `examples/sample_project/qa/fixtures/schema_demo_state_invalid.json`
  - `examples/sample_project/qa/fixtures/schema_demo_trace.json`
- Direct command coverage includes live validation against `qa:schema_state_provider` with `get_qa_schema_demo_state`, saved `--input` validation, trace validation with `--trace` and `--state-path samples[*].state`, and an expected schema mismatch that must exit nonzero with `qa_state_schema_validation_failed` and per-error `json_path`, `schema_path`, `expected`, `actual`, and `actual_value_preview`.

Live-script wait retry and timeout guidance (S65):

- `session.wait_frames` live requests use a frame-derived timeout of `max(5s, ceil(frames / 30fps) + 5s)`, capped at 120 seconds.
- `session run-script` retries only transient transport/resource failures. Bridge-busy diagnostics for an in-progress `session.wait_frames` use wait-aware bounded backoff metadata; timeout, assertion, target, and gameplay failures are not retried.
- Required validation command: `cargo run --bin godot-qa -- session run-script qa/live_scripts/long_wait_smoke.yaml --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario run sample_main_menu_start --project examples/sample_project --json`

Fastest live frame pacing validation:

- `session start --fastest [--fixed-fps N]` and live-script `session_start.frame_pacing` are opt-in launch-time pacing controls; default live sessions remain realtime.
- Unit coverage should prove CLI parsing, live-script object and shorthand parsing, invalid `fixed_fps` validation, generated Godot argv order before `--path`, and reuse mismatch failure for active sessions with different pacing.
- Runtime validation:
  - `cargo run --bin godot-qa -- session start --project examples/sample_project --scene res://ui/main_menu.tscn --headless --fastest --fixed-fps 60 --json`
  - `cargo run --bin godot-qa -- session run-script qa/live_scripts/long_wait_fastest_smoke.yaml --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- session stop --project examples/sample_project --json`

Scenario profiling validation (S46):

- Required repo commands for profile contract validation:
  - `cargo test`
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo run --bin godot-qa -- scenario profile sample_main_menu_start --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario profile sample_setup_steps --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario run sample_main_menu_start --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario list --project examples/sample_project --json`
- Inspect `data.artifacts.profile` and the referenced `profile.json` when debugging profile regressions. It should contain bounded frame samples, monitor counter samples when Godot exposes them, and per-step timing entries.
- Profiling validation must remain non-privileged: do not require Linux `perf`, root permissions, host tracing, or performance thresholds for the default profile command.
- External projects may be used as optional advisory downstream stress validation for scenario listing or known slow scenarios. It is not a required fixture, CI gate, or replacement for the sample-project profile commands.

Experimental editor profiler export validation:

- Unit and CLI coverage must prove `godot-qa profile editor export --project <project> --json` handles missing `.qa/artifacts/editor-profiler/latest.json` as `editor_profile_missing`, malformed artifacts as `editor_profile_invalid`, valid `editor_script_functions` rows, and bounded retained `frames`.
- Parser coverage should include raw Godot editor debugger arrays for `servers:function_signature`, `servers:profile_frame`, and `servers:profile_total`.
- Manual validation requires the Godot Editor: enable the `Godot QA` plugin, run the project, use the `Godot QA Profiler` panel, then run `godot-qa profile editor export --project examples/sample_project --json`.
- This export remains an additional editor-only source. It does not replace `session profile`, `scenario profile`, `check --tests --profile`, or opt-in runtime `top_script_methods`.

Runtime time acceleration validation (S59):

- Use `time_control` only for opt-in scenarios or manifest tests where seconds-based waits, timers, or slow setup would otherwise dominate wall-clock validation time.
- Keep unaccelerated regression runs for flows whose player-facing behavior, screenshots, audio timing, or physics precision is the core contract.
- Warning categories such as `time_control_physics_pairing_missing` are evidence to inspect, not baseline approval and not a replacement for assertion failures.
- Godot `Engine.time_scale` does not accelerate audio playback speed, so audio-sensitive checks should avoid acceleration unless the mismatch is explicitly acceptable.
- High `Engine.time_scale` values can reduce physics precision unless paired with `Engine.physics_ticks_per_second` and `Engine.max_physics_steps_per_frame`; physics-sensitive scenarios should set both paired fields.
- Screenshot-heavy and visual-risk runs can use acceleration for setup, but baseline approval remains manual and diagnostic screenshot artifacts must not update `qa/baselines`.
- Required repo commands for S59 validation:
  - `cargo test repo_contract_time_control_docs`
  - `cargo test`
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo run --bin godot-qa -- scenario run accelerated_timer_demo --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- check --tests --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario run sample_main_menu_start --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario profile sample_main_menu_start --project examples/sample_project --json`

Audio capture and analysis validation (S64):

- Prefer deterministic project-owned export hooks for generated music and SFX. Hooks should be `qa_` methods on durable `qa:` targets and return either bounded RIFF/WAVE bytes/base64 or a dictionary pointing to a WAV under the controlled audio artifact directory.
- Bus capture is optional runtime capability. Tests for unavailable buses or unsupported recording APIs should assert `audio_capture_unavailable` or `audio_capture_failed` with the requested bus and artifact context, rather than treating missing capture as a pass.
- Silent or suppressed QA-mode audio should remain visible through evidence. Validators should inspect `data.audio_analysis` and the `audio/<name>.analysis.json` artifact for high `silence_ratio`, `near_silence: true`, or near-zero `rms`.
- Security negative tests should run real scenarios and inspect JSON failures for stable types, command, target/method, expected/actual values, and artifact paths. Do not use shell fallback logic to turn any nonzero run into success.
- Manual listening remains a separate review step. Audio analysis can flag measurable facts such as silence, clipping risk, loop-boundary delta, transient count, and coarse spectral energy, but it must not judge subjective quality or approve baselines.
- Required S64 negative-path coverage:
  - `cargo test -p godot-qa-cli` (audio export security, capture-unavailable, and silence-analysis unit coverage)
  - `cargo run --bin godot-qa -- scenario lint --project examples/sample_project --json`

Live audio capture and analysis validation (S70):

- Public contract guard: keep `cargo test -p godot-qa-cli repo_contract_live_audio_docs` passing so the documented `audio capture`, `audio analyze`, `audio export`, `audio.capture`, `audio.export`, warning IDs, live artifact layout, window defaults, and validation commands stay visible.
- Addon/protocol symbol guard: keep the GDScript addon's audio bridge unit/integration coverage passing so bridge routes, runtime command cases, controlled `audio_dir`/`wav_path` injection, `max_bytes`, `host_microphone_capture: false`, and `qa_` export method enforcement remain aligned.
- Live validation should start a deterministic sample scene, run bounded capture with `--live --seconds 3 --bus Master --name live_audio_demo --analyze --json`, run standalone `audio analyze .qa/artifacts/live/latest/audio/live_audio_demo.wav --json`, run project-owned `audio export --target qa:audio_controller --method qa_export_audio_track --args '["deterministic_transients"]' --name exported_transients --analyze --json`, and stop the session.
- Treat bus-capture unavailable cases as structured evidence. A runtime without mixed-bus capture should return `audio_capture.unavailable` or `audio_capture.bus_not_found` with the requested bus, command, expected/actual values, and artifact context; tests should not hide that case with shell fallback logic.
- Validate analysis artifacts for `window_seconds = 1.0`, `hop_seconds = 0.5`, `peak_by_window`, `rms_by_window`, `transients_by_window`, `suspected_event_spam_regions`, `clipping_regions`, `silence_regions`, and the absence of raw sample arrays. Subjective sound quality remains outside the CLI pass/fail contract unless an explicit threshold or `--strict-warnings` is used.
- Required S70 validation commands:
  - `cargo test -p godot-qa-cli repo_contract`
  - `cargo test -p godot-qa-cli repo_contract_live_audio_docs`
  - `cargo run --bin godot-qa -- session start --project examples/sample_project --scene res://ui/fixtures/live_audio_demo.tscn --headless --fastest --fixed-fps 60 --json`
  - `cargo run --bin godot-qa -- audio capture --project examples/sample_project --live --seconds 3 --bus Master --name live_audio_demo --analyze --json`
  - `cargo run --bin godot-qa -- audio analyze .qa/artifacts/live/latest/audio/live_audio_demo.wav --json`
  - `cargo run --bin godot-qa -- audio export --project examples/sample_project --target qa:audio_controller --method qa_export_audio_track --args '["deterministic_transients"]' --name exported_transients --analyze --json`
  - `cargo run --bin godot-qa -- session stop --project examples/sample_project --json`

Live session profiling and PATH warning validation:

- Required repo commands:
  - `cargo test`
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo run --bin godot-qa -- session start --project examples/sample_project --headless --json`
  - `cargo run --bin godot-qa -- session profile --project examples/sample_project --frames 10 --json`
  - `cargo run --bin godot-qa -- session stop --project examples/sample_project --json`
- Inspect `data.artifacts.profile` for live profiles. It should contain bounded frame samples, monitor samples, node process counts, resource activity status, and opt-in `top_script_methods`. The sample project instruments `MainMenu._process`, so a live profile of the sample main scene should include that label.
- Inspect `data.profile.top_script_methods` for `scenario profile sample_main_menu_start --project examples/sample_project --json` as well. The one-shot profile path should expose the same opt-in script-span surface and write it to the run-level `profile.json`.

S54 live session profiling retry validation:

- Unit/doc contract: `cargo test live_profile_retry_docs_contract` verifies CLI, artifact, runtime protocol, and test-strategy docs define retry metadata wording, failure kinds, artifact preservation, and agent guidance.
- Bridge-busy retry fixture: `cargo test live_profile_bridge_busy_retries` should simulate transient `session_bridge_busy` transport/resource contention before sampling begins, then assert bounded retry metadata, backoff timing, final status, and success/failure behavior.
- Required live sample-project command: `cargo run --bin godot-qa -- session profile --seconds 3 --max-frame-samples 120 --json`.
- Regression command: `cargo run --bin godot-qa -- session list --json`.
- Agents may retry `session_bridge_busy` because it represents transient live bridge contention before profiling starts. Agents should not retry `session_unreachable`, `session_connect_timeout`, `session_token_invalid`, `profile_result_invalid`, or `artifact_write_failed`; diagnose session configuration, token state, malformed runtime payloads, or filesystem/artifact paths instead.
- Failure diagnostics must preserve distinct types: `session_bridge_busy`, `session_unreachable`, `session_token_invalid`, `session_connect_timeout`, `profile_result_invalid`, and `artifact_write_failed`.
- Unit coverage should keep `doctor` PATH command-surface drift as a warning-only condition by default: top-level `ok` remains true and `failures` remains empty when no hard environment failures exist. `doctor --strict-binary-surface` is the opt-in CI mode that promotes incompatible duplicate command surfaces to `binary_surface_mismatch` while preserving unrelated doctor failures.
- Binary-surface tests may create fake executable `godot-qa` files in temporary directories. They should verify `data.binary_surface` path ordering, `first_path_binary`, version/help probe metadata, structured warnings, and malformed or incomplete help output without depending on the developer machine having stale binaries installed.
- Focused fake-`PATH` tests should assert that duplicate warnings include `type: "multiple_godot_qa_binaries"`, `severity: "warning"`, candidate path/order fields, missing or extra command lists, probe timeout/error metadata when applicable, and remediation text for stale `~/.cargo/bin`, unpacked release archive directories, and repo-local `target/debug` or `target/release` binaries.
- Tests must preserve the S53 non-goals: `doctor` never edits `PATH`, deletes discovered binaries, executes shell snippets, or relies on aliases/functions while probing duplicates. Strict-mode tests should assert only the opt-in failure promotion to `binary_surface_mismatch`.

Display backend and version probe validation guidance (S43):

- Required sample-project validation commands:
  - `cargo run --bin godot-qa -- doctor --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario run visual_occlusion_demo --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario run sample_main_menu_start --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- import --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- check --project examples/sample_project --all-scenes --json`
- Unit and integration coverage should prove prefixed version probes can fall back to direct `godot --version`, while both-probe failure remains `godot_version_probe_failed`.
- Screenshot-backed scenario coverage should assert `launch_display` metadata for selected headless/windowed mode, display driver, virtual-display use, and fallback reason.
- No-display coverage should assert screenshot-backed scenarios fail early as `display_backend_unavailable` when no real display backend or safely detected virtual display wrapper is available; non-screenshot scenarios may remain headless.
- External projects may be used for optional advisory validation only and must not become a required fixture, required CI input, or replacement for the sample-project commands above.

Temporal assertion validation guidance (S33/S52):

- Unit: validate CLI/Scenario parsing for `max_delta_position`, `follows`, `never`, and `within_limits`, including bounded-window exclusivity and sampling defaults.
- Scenario: run pass/fail temporal scenarios that exercise each assertion name and verify actionable failure payload fields.
- Live: run equivalent `godot-qa assert ... --live --json` checks to verify shared runtime behavior rather than separate assertion logic.
- Regression: ensure existing non-temporal assertions and result-envelope fields remain stable.
- Evidence inspection: failing temporal tests should confirm referenced temporal evidence artifact paths exist and include bounded sample context around the failing sample.
- Observe-until unit/doc contract: run `cargo test temporal_observe_until_docs_contract` to keep the documented `assert.observe_until` command, sample scenario names, validation fields, and evidence field names aligned with implementation.
- Observe-until scenario pass: run `cargo run --bin godot-qa -- scenario run temporal_observe_until_demo --project examples/sample_project --json`.
- Observe-until failure contract: run `cargo test temporal_observe_until_failure_contract`, which covers the late-match payload from `temporal_observe_until_late_failure_demo`.
- Observe-until evidence inspection: manually inspect the referenced `artifacts.temporal_evidence` JSON and confirm it records `condition`, `window`, `sample_every_frames`, `observed_samples`, and either `first_matching_sample` or the nearest failing/late boundary sample (`first_late_sample`) with `why_no_match`.

Proximity assertion validation guidance (S34):

- Unit/doc contract: verify parser/docs contracts for `assert.distance`, `distance_changed`, coordinate-space mismatch behavior, and failure payload fields remain stable.
- Scenario: run pass/fail scenarios that cover within-radius (`max`) and outside-radius (`min`) checks using explicit locators such as `qa:actor`.
- Temporal proximity: run `distance_changed` pass/fail scenarios to validate bounded-window threshold behavior.
- Live parity: run equivalent live assertions to confirm one-shot and live modes share runtime behavior.
- Required sample-project commands:
  - `cargo run --bin godot-qa -- scenario run proximity_assertions_demo --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario run proximity_assertions_failure_demo --project examples/sample_project --json || true`
  - `cargo run --bin godot-qa -- assert distance qa:actor qa:target --project examples/sample_project --max 96 --json`

Surface probe assertion validation guidance (S36):

- Unit/doc contract: verify parser/docs contracts for `assert surface-probe` and `assert surface-matches`, including explicit provider targets where required, QA-prefixed provider/state methods or metadata, bounded point/radius semantics, and stable failure payload keys.
- Scenario: run pass/fail probe scenarios that cover declared surfaces (`walkable`, `blocked`, etc.), unknown-label failure, unsupported coordinate-space failure, and out-of-bounds request failure.
- Live/assertion parity: run equivalent live assertions to confirm one-shot and live paths share the same runtime assertion behavior and evidence fields.
- Sample-project provider coverage: ensure sample fixtures prove surface outcomes come from explicit QA provider methods rather than collision-name inference.
- Required sample-project commands:
  - `cargo test`
  - `cargo run --bin godot-qa -- scenario run surface_probe_demo --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario run surface_probe_failure_demo --project examples/sample_project --json || true`
  - `cargo run --bin godot-qa -- assert surface-matches qa:demo_actor --project examples/sample_project --snapshot after_move --json`
  - `cargo run --bin godot-qa -- check --project examples/sample_project --tests --json`

## Android live-session verification

Automated Android coverage requires neither an SDK nor a device. Fake Godot and
ADB executables exercise desktop defaults and conflicts, preset/package
resolution, device selection states, exact serial-scoped commands, private stdin
bootstrap provisioning, token redaction, stage failures and cleanup, old
desktop metadata, target-aware liveness, binary artifact pulls, nested path
rewriting, traversal rejection, and desktop live regressions. Live edit over the
Android transport has a dedicated unit test: an `edit.set_property` round-trip
through the forwarded bridge returns the desktop-identical ledger entry
unchanged, proving scalar edit params and results pass through with no adb
artifact pull or file-transport fallback.

The Android APK cache is covered with a fake Godot: key stability for an
unchanged project and sensitivity to file/preset/package/version changes, the
hit/miss/bypass paths (including `--no-apk-cache`), newest-four eviction, and
degradation to a fresh export when an entry is corrupt or a version probe fails.

Android one-shot `scenario run` verification also needs neither an SDK nor a
device. Unit tables cover option rejection, the scenario preflight
(`android_scenario_unsupported` reasons), the request rewrite (device paths,
stripped frame budget), the echoed-artifact substitution, and the collection walk
(evidence pulled, run-level artifacts substituted, progress rewritten). A full
post-deploy loop drives `execute_android_scenario_on_session` against a fake TCP
bridge and fake ADB (built from the same executable pattern), asserting a
desktop-identical verdict, host artifacts, `target: "android"`, and the exact
cleanup command order; sibling cases pin the round-trip failure classification and
a failed cleanup step. Hardware acceptance runs a sample scenario end-to-end on
the dedicated package.

Hardware acceptance is serialized against the dedicated sample package:
export/install/start; status and core inspection/input/assertion commands;
profile/log/error retrieval; screenshot and snapshot materialization; a live-edit
hide → screenshot → `export-patch` → reload-restores sequence (the packaged
reload restores the node and clears the edit ledger); then stop.
It verifies nonblank parseable host artifacts, application termination, exact
forward removal, private bootstrap/session cleanup, local metadata cleanup, and
token absence from logcat and artifacts. The dedicated package is uninstalled at
the end.

Scenario lint validation guidance (S37):

- Unit/doc contract: verify `scenario lint` syntax, warning-vs-strict exit behavior, suppression-reason requirements, and stable `data.findings` / `data.suppressed_findings` JSON keys.
- Scenario: lint all scenarios and one named scenario to confirm project-wide and scoped behavior.
- Strict mode: confirm warning-only lint findings exit `0` by default and non-zero under `--strict`.
- Fixture policy: intentionally bad lint fixtures are valid test assets, but they should not be included in happy-path `scenario run` regression gates unless the gate explicitly expects warnings.
- Required sample-project commands:
  - `cargo run --bin godot-qa -- scenario lint --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario lint --project examples/sample_project --scenario sample_main_menu_start --json`
  - `cargo run --bin godot-qa -- scenario lint --project examples/sample_project --scenario lint_fixture_masking_demo --json`
  - `cargo run --bin godot-qa -- scenario lint --project examples/sample_project --scenario lint_fixture_masking_demo --strict --json || true`

## Golden screenshot policy

- Golden screenshots are reviewed assets.
- Agents may create candidate baselines under `.qa/artifacts`.
- Agents must not update `qa/baselines` unless explicitly asked.
- S9 uses RGBA different-ratio thresholding with explicit diff artifacts; masks/per-region tolerances remain later work.
- Low-information screenshot coverage must stay active for screenshot-backed visual assertions, including `visible_pixels`; blank, transparent, tiny, or visually flat evidence fails before any baseline approval flow.
- Visual assertion limitations are intentional: use them for deterministic 2D/UI rendered-pixel evidence, and use layout, tree visibility, or manual screenshot baseline review when those contracts are a better fit.
- Assertion choice guidance:
  - use scene-tree `visible`/`hidden` for effective `CanvasItem` visibility state
  - use layout assertions for geometry and overlap contracts that do not require rendered-pixel proof
  - use visual pixel assertions (`visible_pixels`, `not_occluded`, `renders_above`, `renders_below`) for screenshot-backed rendered evidence with explicit pixel thresholds/evidence fields
  - use `saved_snapshot_pixels` when an already saved snapshot contains real screenshot pixels and viewport-space target evidence that should be measured without replaying the scenario
  - use screenshot baseline comparison for full-frame visual regression review, with manual baseline approval only

S44 visual evidence validation guidance:

- Unit/doc contract: verify documentation names `qa_visual_rect`, `qa_set_visual_visible`, `Line2D`, `unsupported_visual_assertion`, `unsupported_coordinate_space`, `screenshot_capture_failed`, real rendered pixels, and non-visual state/probe alternatives.
- Runtime-backed positive coverage: run `visual_line_indicator_demo` and confirm a deterministic `Line2D` target produces non-zero visible-pixel evidence from the rendered stroke bounds.
- Coordinate-space negative coverage: run `visual_coordinate_space_failure_demo` and verify the failure payload includes the assertion command, target, requested space, actual space when known, and remediation guidance.
- Headless negative coverage: run `visual_headless_failure_demo` with `--headless` and verify it fails because real rendered pixels are unavailable, not because of generic missing screenshot content.
- Occlusion regression coverage: run `visual_occlusion_demo`, `visual_occlusion_failure_demo`, and the direct `assert visible-pixels qa:important_actor` command so screenshot, mask, and overlay artifacts stay attached to supported target failures.

S47 visual preflight and saved-snapshot pixel validation guidance:

- Required validation commands:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo run --bin godot-qa -- visual preflight --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario run saved_snapshot_pixels_demo --project examples/sample_project --json`
  - `cargo test saved_snapshot_pixels_failure_contract`
  - `cargo run --bin godot-qa -- scenario run visual_line_indicator_demo --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario run visual_occlusion_demo --project examples/sample_project --json`
- Preflight coverage should assert `visual_support.real_viewport_pixels`, `target_hidden_capture`, `mask_generation`, and `saved_snapshot_object_pixels`, including forced-headless and no-display unavailable reasons.
- Saved-snapshot pixel coverage should prove a real saved screenshot with target `visual_evidence` can satisfy count or ratio thresholds, while blank/low-information screenshots, unresolved locators, unsupported target geometry, and threshold mismatches fail with stable payloads.

Scenario bundle validation guidance (S38):

- Unit/doc contract: verify docs and command contracts for `scenario bundle list` and `scenario bundle run` remain stable, including bundle manifest keys and declarative-only restrictions.
- Parallel execution contract: verify `scenario bundle run --jobs 0`, `--jobs 2`, and `--jobs -1` expose `data.parallelism`, preserve stable `data.runs[]` ordering, run all runnable children even after required child failures, and keep retry attempts isolated per child.
- Selection and dispatch contract: verify `--only` and `--failed-only` narrow the run without changing `data.runs[].index` or the verdict of a scenario that did not run, that an unknown `--only` name and a missing `--failed-only` baseline exit 2 without writing a run directory, that an empty selection exits 0 with a `selection_empty` warning inside `summary.json`, and that `--schedule`/`scenario.schedule` are reported in `data.scheduling` while `data.runs[]` stays in expansion order.
- Latest-link contract: verify `.qa/artifacts/bundles/<bundle>/latest` is a relative symlink on Unix or NTFS directory junction on Windows to the newest run, is rotated on every run, and survives `artifacts.keep_runs` pruning without dangling.
- Multi-bundle contract: verify `scenario bundle run smooth_gate lobby_gate race_regression_gate` reports one envelope (`data.bundles[]`, `data.summary`, failures stamped with `bundle`), that each named bundle still writes its own run directory while `warm_pool` appears only once at the envelope level, that `--warm` seeds one pool and takes one skip decision on the summed runnable count, and that a duplicated name or `--equivalence-check`/`--only`/`--failed-only` with several names exits 2 as `invalid_arguments` without writing anything.
- Fixture policy: keep exactly one intentionally failing bundle fixture (`malformed_bundle_demo`) for actionable negative validation without making normal bundle discovery fail.
- Required sample-project commands:
  - `cargo run --bin godot-qa -- scenario bundle list --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario bundle run malformed_bundle_demo --project examples/sample_project --json || true`
  - `cargo run --bin godot-qa -- scenario lint --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario run sample_main_menu_start --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario run sample_settings_layout --project examples/sample_project --json`
- Regression contract: `scenario lint` coverage remains separate, and direct `scenario run` regressions remain the source behavior for child scenarios executed through bundle runs.

Recorder negative suggestion validation guidance (S39):

- Unit/doc contract: verify `scenario record` docs and parser contracts include bounded replay flags (`--input-script`, `--duration-frames`), duplicate-name/output rejection, JSON validation failures for malformed input scripts, and manual review wording.
- Runtime-backed bounded replay: run deterministic fixture input scripts and verify bounded recorder mode exits on its own, emits candidate/review artifacts, and preserves empty top-level `assert`.
- Negative suggestion gating: confirm suggestions are emitted only when `kind`, `assertion`, `input_moment`, `target`, `snapshots`, and concrete runtime `evidence` are present.
- Manual review contract: confirm docs and tests preserve accept/reject/edit workflow language and that suggestions complement `scenario lint` instead of replacing it.
- Baseline safety contract: confirm recorder suggestions remain artifacts-only and do not auto-approve or write files under `qa/baselines`.
- Gamepad capture contract: unit-cover the export-to-candidate conversion for `input_joy_button` (tap shorthand vs `frames` holds, non-default `device`) and `input_joy_motion` (axis + quantized value), assert malformed joypad events are skipped instead of emitting broken steps, and reject out-of-range `--joy-motion-*` thresholds before launch (`invalid_joy_motion_options`). The quantization itself lives in `qa_recorder.gd` (bucket crossing, deadzone snap-to-zero, per-axis minimum frame gap, final-value flush at export); a real-gamepad round-trip needs human hardware and stays a manual check.
- Required spec commands:
  - `cargo test`
  - `cargo run --bin godot-qa -- scenario record --project examples/sample_project --name recorder_negative_suggestions_demo --scene res://ui/fixtures/recorder_negative_suggestions.tscn --input-script qa/recordings/negative_suggestions_input.json --duration-frames 45 --json`
  - `cargo run --bin godot-qa -- scenario lint --project examples/sample_project --scenario recorder_negative_suggestions_demo --json`
- Regression commands:
  - `cargo run --bin godot-qa -- scenario record --project examples/sample_project --name recorder_basic_review_demo --scene res://ui/main_menu.tscn --duration-frames 5 --json`
  - `cargo run --bin godot-qa -- scenario list --project examples/sample_project --json`

Scenario run result mapping validation guidance (S45):

- Unit/doc contract: verify `scenario run --json` retains separate `data.process`, `data.runtime_result`, `data.logs`, and `data.outcome` summaries, and that strict default semantics still require process exit `0`, runtime `ok: true`, empty runtime failures, and clean relevant logs.
- Strict process mismatch coverage: run `process_exit_strict_failure_demo` directly (do not paper over the nonzero exit with shell fallback logic); it must prove a clean runtime result plus non-zero process exit fails as `godot_run_failed`, records process details, and sets `data.outcome.reason: "godot_run_failed"`.
- Wrapper-warning policy coverage: run `process_exit_warning_policy_demo` and verify only explicit `process_outcome_policy.known_wrapper_exit_codes` can produce `data.outcome.status: "pass_with_warning"`, top-level `ok: true`, empty top-level `failures`, and warning-level details under `data.process.warnings`.
- Missing and malformed result coverage: verify absent runtime `result.json` remains `scenario_result_missing` with `data.runtime_result.status: "missing"`, and malformed or structurally invalid payloads remain `scenario_result_invalid` with `data.runtime_result.status: "invalid"`.
- Runtime assertion failure coverage: verify runtime failures remain failures even when the Godot process exits `0`, with `data.runtime_result.status: "failed"` and the runtime failure type preserved as the outcome reason.
- Log-error distinction coverage: verify Godot log errors and warnings remain distinct from process failures and runtime assertion failures through `data.logs.status`, `error_count`, `warning_count`, and `godot_log_error` or `godot_log_warning` failure types.
- Warm engine-log parity coverage: verify a warm (pooled-session) run reports engine log errors identically to cold — the run's `logs.txt` is materialized from the session log window bounded by the `[godot-qa] scenario-log begin|end <run-id>` markers, `data.warm.log_slice.status` is `sliced`, each `godot_log_error` carries `warm_log_window`, and an addon without the markers degrades to `data.logs.status: "missing"` plus one bundle-level `warm_log_markers_missing` warning. Also verify engine lines emitted outside every scenario window (session start, idle, shutdown) are summarized as `data.warm.session_log_noise` with one advisory `warm_session_log_noise` warning, and never become a failure.
- Project log-ignore coverage: verify a project's `logs.ignore` patterns silence matching engine `ERROR:`/`WARNING:` lines on top of the built-in ignores, that the identical list reaches the cold scan, the warm per-scenario scan, and the warm session-noise scan (`data.warm.session_log_noise`), that the silenced lines are counted as `data.logs.ignored_count` with `data.logs.ignore_patterns_applied` reporting how many project patterns were in force, and that the very same log without the patterns still fails as `godot_log_error`.
- Bundle regression: `scenario bundle run` remains strict and delegates child behavior to direct `scenario run`; wrapper-warning behavior is not a bundle-level policy unless separately documented and tested.
- Required sample-project commands:
  - `cargo run --bin godot-qa -- scenario run sample_main_menu_start --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario run process_exit_strict_failure_demo --project examples/sample_project --json || true`
  - `cargo run --bin godot-qa -- scenario run process_exit_warning_policy_demo --project examples/sample_project --json`
- Regression commands:
  - `cargo run --bin godot-qa -- scenario run sample_setup_steps --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario bundle run sample_regression_gate --project examples/sample_project --json`

Scenario bundle validation guidance (S50):

- Composition contract: sample and downstream gates may use `required_bundles` for bounded declarative reuse, but bundles remain declarative data only and are not a CI scripting language.
- Expansion metadata: validate bundle summaries for `source_bundle`, `bundle_chain`, `run_root`, `godot_args`, `launch_display`, and per-child pass/fail status.
- Failure coverage: missing included bundles, unsafe bundle names, duplicate required scenarios, and recursive-bundle cycles such as `recursive_bundle_demo` must fail before any child scenario runs with actionable JSON failures.
- Godot args preservation: child scenarios launched from composed bundles must preserve scenario-owned `godot_args` exactly as direct `scenario run` would.
- Optional behavior: missing optional scenarios remain structured warnings by default and become failures only under `--strict`.
- Sample commands:
  - `cargo run --bin godot-qa -- scenario bundle list --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- scenario bundle run composed_regression_gate --project examples/sample_project --json`

Scenario bundle validation guidance (S51):

- Unit/doc contract: `cargo test -p godot-qa-cli s51_bundle_display_isolation_docs_contract` verifies CLI, artifact, scenario, architecture, and test-strategy docs define bundle display isolation metadata, infrastructure failure kinds, retry policy constraints, and sample gates.
- Startup failure contract: `cargo test display_startup_failure_contract` should cover `display_startup_exhausted`, `virtual_display_allocation_failed`, `stale_process_cleanup_failed`, and infrastructure-classified `scenario_result_missing` after display startup failure.
- Screenshot-heavy regression gate: run `cargo run --bin godot-qa -- scenario bundle run screenshot_heavy_regression_gate --project examples/sample_project --json` to verify screenshot-backed child scenarios keep passing inside a bundle without in-scenario display workarounds.
- Sample regression gate: run `cargo run --bin godot-qa -- scenario bundle run sample_regression_gate --project examples/sample_project --json` to preserve the existing bundle regression contract.
- Forced display-startup fixture inspection: inspect a controlled failure artifact and confirm `summary.json` includes attempts, `display_lifecycle`, `cleanup_status`, `retry_guidance`, launch metadata, run roots, and a distinct infrastructure `category`.

Hidden-overlay live inspection gate validation guidance (S49):

- Unit/doc contract: verify `live-gate list` and `live-gate run` docs preserve the CLI command contract, `qa/live_gates/` manifest directory, artifact evidence wording, runtime bridge command list, and declarative manifest boundaries.
- Runtime contract: live gates must use normal live sessions and documented bridge commands, verify overlay-hidden evidence through project-owned QA surfaces, and fail with actionable diagnostics for stale sessions, runtime errors, rejected methods, probe failures, and missing required anchors.
- Optional-anchor contract: missing optional anchors emit warnings by default and become failures under `--strict`; missing required anchors must remain failures in both modes.
- Required validation commands:
  - `cargo test`
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo run --bin godot-qa -- live-gate list --project examples/sample_project --json`
  - `cargo run --bin godot-qa -- live-gate run hidden_overlay_inspection --project examples/sample_project --json`
  - `cargo test hidden_overlay_missing_anchor_contract`
- Regression validation commands:
  - `cargo run --bin godot-qa -- session start --project examples/sample_project --scene res://ui/main_menu.tscn --json`
  - `cargo run --bin godot-qa -- scenario run sample_main_menu_start --project examples/sample_project --json`

Downstream tool-improvement hardening validation guidance (S77):

- Unit/doc contract: verify screenshot capture setup scripts, audio `stop_after_state`, empty profile selection, extended `session profile --frames`, and live bridge retry wording remain documented and tested.
- Runtime contract: `audio capture --stop-after-state` uses the same read-only `state_matches` predicate surface as assertions and must still write bounded WAV evidence when the predicate times out.
- Retry contract: live audio capture and other direct live evidence commands attach bounded retry metadata for transient `session_bridge_busy`/resource-busy failures without adding a general bridge queue.
- Required validation commands:
  - `cargo test`
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo run --bin godot-qa -- check --project examples/sample_project --tests --json`
# External-wrapper verdict regression

`artifact_verdict_cli.rs` exercises a wrapper that reports zero around a failing child, a matching successful pair of receipts, stale/malformed/missing receipts, a signal receipt, child and wrapper timeouts, and exclusive run creation. The owned-process fixture writes a completed result, leaves an inherited output handle in a descendant, and stalls during teardown; the runner must stop the owned process group after three seconds and keep its output evidence. These are local fake-child tests and do not require a rendered display.

## Consumer regression and screenshot performance checks

The scaled locator-click fixture creates its UI through Godot APIs and asserts
actual `Button.pressed` signals at 720p, scaled 1080p, with letterboxing, resizing,
and a transformed CanvasLayer. The opt-in Cargo test stages an isolated project.
For a Linux display large enough to exercise each requested window size:

```bash
xvfb-run -a -s '-screen 0 2560x1440x24' env GODOT_QA_VISUAL_TESTS=1 GODOT_BIN="$(command -v godot)" cargo test -p godot-qa-cli --test scaled_locator_click_runtime -- --nocapture
godot --headless --path examples/sample_project --script res://qa/tests/snapshot_image_equivalence.gd
cargo test -p godot-qa-scenario --release screenshot_analysis_repeated_targets_benchmark -- --ignored --nocapture
```

Run `cargo xtask prepare-sample` first. The image-equivalence fixture compares
decoded fallback PNG pixels with the former algorithm at 14 sizes, including
overlapping borders, irregular tiles, 720p, 1080p, and 4K, and prints separate
image-generation timings. The ignored Rust benchmark checks identical repeated
target evidence and measures shared analysis at 720p, 1080p, and 4K. Keep machine
load and build profile with reported numbers; these microbenchmarks do not
establish whole-suite speedups and impose no timing threshold on normal CI.

Process regressions cover native signals separately from tool timeouts,
nonce-bound stale/partial completion, and the existing full shutdown grace.
Bundle regressions reuse prepared inputs after their source files disappear,
check fresh-invocation invalidation, and preserve selection/strict failure behavior.
Visual isolation tests inject errors at every capture phase and during cleanup,
and compare unchanged as well as changed image bounds and pixels.
