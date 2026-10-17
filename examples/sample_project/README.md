# Sample Project

`examples/sample_project` is the checked-in Godot project for reproducible smoke checks and feature examples.

From the repository root, run `cargo xtask prepare-sample` before importing or opening it. This copies the canonical addon into the ignored `examples/sample_project/addons/godot_qa` directory on Linux and Windows. It also prepares the native process-exit fixture under `.qa/tools/` for isolated process tests, without changing project launch configuration. Repeat after changing addon source; edit `addons/godot_qa` at the repository root, not the generated copy.

Install Godot 4.5.2 and the CLI as described in the [README](../../README.md). Run the commands below from the repository root with `--project examples/sample_project`, or from this directory without that option. Screenshot scenarios require a working display. Some fixtures intentionally fail; use the named happy-path scenarios for a first run.

On Windows, the sample uses Godot's ANGLE compatibility driver (Direct3D 11), including in CI. Other projects need a working rendering backend appropriate to their renderer and hardware.

Scenes:
- `res://ui/main_menu.tscn`: entry scene with `qa:start_button`, `qa:settings_button`, explicit focus neighbors for `focus graph`, an instanced pause menu, and a start-button layout aligned to coordinate-click smoke checks.
- `res://ui/pause_menu.tscn`: reusable pause overlay with `qa:pause_menu` and `qa:resume_button`.
- `res://ui/settings_menu.tscn`: responsive layout sample with `qa:settings_menu`, `qa:language_button`, and `qa:apply_button`.
- `res://ui/fixtures/audio_demo.tscn`: deterministic generated loop fixture with `qa:audio_controller` for audio export and Master bus capture demos.
- `res://ui/fixtures/live_input_probe.tscn`: focused input probe with `qa:input_probe_button` and `qa:input_probe_result` for keyboard and joypad live-input checks.
- `res://ui/fixtures/text_entry_probe.tscn`: focused LineEdit with `qa:text_entry_field` and `qa:text_entry_result` for `input type` / unicode key live-input checks.
- `res://ui/fixtures/runtime_error_probe.tscn`: session-only fixture that emits one structured QA runtime error for `errors` smoke coverage.
- `res://ui/fixtures/physics_overlay_demo.tscn`: 2D collider overlay fixture with visible shapes and visual-footprint metadata. QA IDs: `qa:overlay_actor` for aligned body/visual bounds, `qa:offset_collider_actor` for a shifted collider, `qa:oversized_collider_actor` for an oversized collider, `qa:trigger_area` for an Area2D trigger, and `qa:decorative_padding_actor` for non-solid decorative padding.
- `res://ui/fixtures/surface_audit_demo.tscn`: provider-driven surface audit fixture with `qa:surface_provider`, `qa:clear_spawn`, `qa:blocked_spawn`, `qa:route_start`, `qa:route_mid`, `qa:route_finish`, `qa:left_lane`, `qa:right_lane`, and `qa:centerline` anchors.
- `res://ui/fixtures/bad_settings_menu.tscn`: intentionally broken layout fixture for negative layout-audit checks.
- `res://ui/fixtures/presented_startup_demo.tscn`: static navy scene with four colored stripes for isolated compositor startup capture. `presented_startup_stale.tscn` uses the same script with a test-only no-swap transition: Gamescope keeps the red first frame while the Godot viewport becomes navy. `presented_startup_blank.tscn` verifies strict rejection of low-information captures. The production QA runner keeps its `force_draw(true)` startup fix.

`screenshot assert-startup-presented` is a Linux-only check that owns headless Gamescope and needs a direct Godot launch. It is separate from ordinary viewport screenshot coverage on Linux and Windows. The opt-in integration test exercises Wayland/X11, the no-swap negative, and resize handling. If you configured a local launch wrapper, clear `godot.launch_prefix` in ignored `qa/config.local.yaml` for this command.

Scenario names:
- `sample_main_menu_start`
- `sample_setup_steps`
- `sample_settings_layout` (declares `execution.isolation: cold`)
- `saved_joypad_input_demo`
- `project_args_demo`
- `visual_occlusion_demo`
- `saved_snapshot_pixels_demo`
- `visual_metadata_rect_demo`
- `visual_line_indicator_demo`
- `surface_probe_demo`
- `surface_matches_temporal_demo`
- `surface_probe_failure_demo`
- `surface_matches_temporal_failure_demo`
- `state_matches_get_named_qa_state_demo`
- `state_matches_array_method_demo`
- `audio_export_hook_demo`
- `audio_capture_demo`
- `physics_overlay_demo`

Cold-isolation scenarios:
- `sample_settings_layout` is the sample project's only `execution.isolation: cold` scenario. Its four-viewport matrix cannot be honoured by a reused warm session, so declaring the opt-out keeps it explicitly exempt from `--warm-strict` enforcement (`data.warm.declared: true`) instead of failing `scenario bundle run sample_regression_gate --warm --warm-strict`. Single-viewport `layout_audit` coverage stays warm-eligible in `sample_layout_single_viewport`.

Audio scenarios:
- `audio_export_hook_demo` calls `qa_export_audio_track` on `qa:audio_controller` and writes `audio/generated_dodgeball_loop.wav` plus `audio/generated_dodgeball_loop.analysis.json`.
- `audio_capture_demo` starts deterministic playback on the `Master` bus and writes `audio/dodgeball_music.wav` plus `audio/dodgeball_music.analysis.json` when runtime bus capture is available. If Godot cannot record the bus in the current environment, the scenario reports an actionable `audio_capture_unavailable` or `audio_capture_failed` failure.

Live gate fixtures:
- `hidden_overlay_inspection`: starts `res://ui/main_menu.tscn`, verifies the project-owned hidden overlay hook, inspects required anchors (`qa:main_menu`, `qa:start_button`, `qa:demo_actor`), and records artifacts under `.qa/artifacts/live-gates/hidden_overlay_inspection/`.

Manifest-backed tests:
- `qa/tests/manifest.json`: explicit `check --tests` contract with one `scene` entry and one `script` entry.
- `qa/tests/script_smoke.gd`: minimal passing `run_qa_test()` fixture for manifest-backed script coverage.
- `qa/tests/joy_lifecycle_test.gd`: direct headless check of virtual controller signal timing, selective button/axis release, unowned-device rejection, and reset/finalization cleanup. Run with `godot --headless --path examples/sample_project --script res://qa/tests/joy_lifecycle_test.gd`.
- `qa/live_scripts/joy_lifecycle_cleanup_smoke.yaml`: headless live-session script that holds a virtual pad across scene replacement, verifies its input and roster were cleared, then stops another held-pad session with before/after cleanup evidence.
- `qa/gates/joy_lifecycle_failure_cleanup.yaml`: deliberately failing two-scenario warm gate. Run with `scenario bundle run joy_lifecycle_failure_cleanup --warm --warm-strict --jobs 1 --schedule manifest --headless`; expect bundle exit 1 from the first false assertion and require `joy_lifecycle_after_fail` to pass in the same session.

Verification commands:

```bash
godot-qa doctor --project examples/sample_project --json
godot-qa check --project examples/sample_project --tests --json
godot-qa scenario list --project examples/sample_project --json
godot-qa scenario run sample_main_menu_start --project examples/sample_project --json
godot-qa scenario run project_args_demo --project examples/sample_project --json
godot-qa scenario run sample_setup_steps --project examples/sample_project --json
godot-qa scenario run sample_settings_layout --project examples/sample_project --json
godot-qa scenario run saved_joypad_input_demo --project examples/sample_project --json
godot-qa scenario run audio_export_hook_demo --project examples/sample_project --json
godot-qa scenario run audio_capture_demo --project examples/sample_project --json
godot-qa layout audit --project examples/sample_project --scene res://ui/settings_menu.tscn --json
godot-qa layout audit --project examples/sample_project --scene res://ui/fixtures/bad_settings_menu.tscn --json || true
godot-qa session start --project examples/sample_project --scene res://ui/settings_menu.tscn --json
godot-qa layout audit-current --project examples/sample_project --scenario sample_settings_layout --live --json
godot-qa session stop --project examples/sample_project --json
godot-qa session start --project examples/sample_project --scene res://ui/fixtures/bad_settings_menu.tscn --json
godot-qa layout audit-current --project examples/sample_project --scenario sample_settings_layout --live --json || true
godot-qa session stop --project examples/sample_project --json
godot-qa session start --project examples/sample_project --scene res://ui/main_menu.tscn --json
godot-qa screenshot capture --project examples/sample_project --out .qa/artifacts/live-manual/main_menu.png --json
godot-qa screenshot compare --project examples/sample_project --live --baseline qa/baselines/live/main_menu.png --json
godot-qa node inspect qa:start_button --project examples/sample_project --json
godot-qa focus graph --project examples/sample_project --json
godot-qa input mouse click --project examples/sample_project --x 640 --y 420 --json
godot-qa assert visible qa:pause_menu --project examples/sample_project --live --json
godot-qa session stop --project examples/sample_project --json
godot-qa session start --project examples/sample_project --scene res://ui/fixtures/runtime_error_probe.tscn --json
godot-qa errors --project examples/sample_project --json
godot-qa session stop --project examples/sample_project --json
godot-qa session start --project examples/sample_project --scene res://ui/fixtures/live_input_probe.tscn --json
godot-qa input key Enter --project examples/sample_project --json
godot-qa assert text_contains qa:input_probe_result key:Enter --project examples/sample_project --live --json
godot-qa input joy button start --project examples/sample_project --json
godot-qa assert text_contains qa:input_probe_result joy:start:device=0 --project examples/sample_project --live --json
godot-qa input joy motion left_x 1.0 --device 0 --frames 2 --project examples/sample_project --json
godot-qa assert text_contains qa:input_probe_result joy:motion:device=0:left_x=1.00 --project examples/sample_project --live --json
godot-qa session stop --project examples/sample_project --json
godot-qa live-gate list --project examples/sample_project --json
godot-qa live-gate run hidden_overlay_inspection --project examples/sample_project --json
```

The checked-in live screenshot baseline is `examples/sample_project/qa/baselines/live/main_menu.png`. Baseline approval remains manual; regenerate candidates under `.qa/artifacts/` first and only replace checked-in baselines deliberately.

Review template examples:

```bash
godot-qa review list-templates --json
godot-qa review init scenario-smoke --project examples/sample_project --name smoke_review --scene res://ui/main_menu.tscn --out .qa/review-scenarios/smoke_review.yaml --json
godot-qa scenario run --file .qa/review-scenarios/smoke_review.yaml --project examples/sample_project --json
godot-qa review init deterministic-runtime-review --project examples/sample_project --name deterministic_review --scene res://ui/main_menu.tscn --project-arg --seed --project-arg 123456 --frames 60 --out .qa/review-scripts/deterministic_review.yaml --json
godot-qa session run-script .qa/review-scripts/deterministic_review.yaml --project examples/sample_project --json
```

These generated files use the same sample project anchors and runtime commands as hand-authored checks. Review templates accelerate exploratory QA without replacing final human review or manual screenshot baseline approval.

Temporal probe fixture:
- `qa_temporal_set_mode(mode: String)` on `qa:main_menu` configures deterministic temporal probe behavior for sample scenarios.
- Supported modes: `steady` (default) and `failure`.
- The fixture is setup-only; temporal movement and follow transitions are still produced by normal `_process` runtime updates on `qa:demo_actor` and `qa:demo_follower`.
- `qa:demo_actor` also updates `qa_temporal_state` each frame (`steady` or `forbidden`) so `never` temporal assertions can be demonstrated deterministically.

Surface probe fixture:
- `qa_probe_surface(request: Dictionary)` on `qa:demo_actor` is an explicit sample-project QA surface provider method.
- `qa_probe_actor_surface_state()` and `qa_current_surface()` on `qa:demo_actor` report actor-owned surface state for `surface_matches`.
- `qa_surface_place_matching()` and `qa_surface_place_mismatch()` on `qa:main_menu` are setup-only fixtures that position `qa:demo_actor` and set deterministic actor-reported state for passing/failing surface checks.
- `visual_metadata_rect_demo` validates metadata-backed visual rect evidence through `qa:metadata_visual_actor`, which exposes `qa_visual_rect()` and `qa_set_visual_visible(visible)`.

Surface audit fixture:
- `qa_surface_probe(request: Dictionary)` on `qa:surface_provider` is the explicit read-only provider for `surface audit`; project semantics stay in this provider.
- `qa:clear_spawn` and `qa:blocked_spawn` cover spawn clearance pass/fail behavior.
- `qa:route_start`, `qa:route_mid`, and `qa:route_finish` cover route/corridor clearance including the narrow corridor case.
- The fixture includes clear spawn, blocked spawn, narrow corridor, dead-end pocket, soft blocker, hard blocker, and mirrored-delta layout cases.
- `qa:left_lane`, `qa:right_lane`, and `qa:centerline` anchor mirrored region comparison.
- Live validation runs `surface_probe_demo` for clear spawn and corridor JSON/heatmap payloads, and the blocked-spawn fixture for the expected strict-warning failure.

State matches fixture:
- `get_raft_motion_qa_state()` on `qa:main_menu` is a zero-argument, read-only-by-convention aggregate QA state method that returns JSON-compatible raft motion/sample gameplay data.
- `state_matches_get_named_qa_state_demo` validates the approved `get_*_qa_state` method naming pattern for `assert.state_matches`.
- `state_matches_array_method_demo` validates array-returning QA state methods plus `approx_equals`, `approximately_equals`, `greater_than`, and `less_than` state checks.

Project argument fixture:
- `project_args_demo` starts `res://ui/fixtures/project_args_probe.tscn` with both `godot_args` and `project_args`.
- The fixture validates that project/game command-line flags such as `--save-slot qa-profile` and `--seed=42` arrive after Godot's `--` separator.

Scenario bundles:
- `sample_regression_gate`
- `reusable_menu_gate`
- `composed_regression_gate`
- `screenshot_heavy_regression_gate`

`composed_regression_gate` uses `required_bundles` to include `reusable_menu_gate`, then runs direct required scenarios including `bundle_godot_args_demo`.
Bundle summaries preserve child source metadata, `godot_args` and `project_args` metadata, launch display metadata, run roots, and per-child status.
Bundle runs write a concise `artifact-index.json` file in addition to `summary.json`, and expose the same child index as `data.artifact_summary`; composed gates retain each child scenario's `source_bundle` and `bundle_chain` lineage in that summary.
`screenshot_heavy_regression_gate` runs the screenshot-backed visual sample scenarios (`visual_occlusion_demo`, `saved_snapshot_pixels_demo`, `visual_metadata_rect_demo`, and `visual_line_indicator_demo`) through the normal bundle runner so summary artifacts preserve child `launch_display` metadata and `run_root` screenshot evidence.
`recursive_bundle_demo` is the single intentional cycle fixture; running it should fail with recursive-bundle failure wording and should not be used as a happy-path gate.

Bundle commands:

```bash
godot-qa scenario bundle list --project examples/sample_project --json
godot-qa scenario bundle run sample_regression_gate --project examples/sample_project --json
godot-qa scenario bundle run composed_regression_gate --project examples/sample_project --json
godot-qa scenario bundle run screenshot_heavy_regression_gate --project examples/sample_project --json
```

## Android live sample

The checked-in `Android QA` export preset is the dedicated debug-live acceptance
target. It uses the unique package `org.godotqa.sample.androidlive`, includes the
Godot QA addon/autoload, and grants Android Internet access for the loopback
bridge.

With Android export templates and one authorized device:

```bash
godot-qa session start --target android --preset "Android QA" --json
godot-qa session status --json
godot-qa session stop --json
```

The sample package is only for QA. Uninstall it after physical-device acceptance.
