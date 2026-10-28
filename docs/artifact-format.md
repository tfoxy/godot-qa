# Artifact Format

Artifacts are the evidence produced by `godot-qa`. They should be easy for humans and AI agents to inspect.

## Directory layout

```text
.qa/artifacts/<scenario>/<run-id>/
  meta.json
  request.json
  result.json
  profile.json
  logs.txt
  snapshots/
    <snapshot-name>/
      screenshot.png
      tree.json
      ui.json
      focus.json
      errors.json
  diffs/
    screenshot-01-<snapshot-name>.diff.png
    screenshot-01-<snapshot-name>.metrics.json
  temporal/
    max_delta_position-01.json
    follows-01.json
    never-01.json
    within_limits-01.json
  diagnostics/
    <assertion-id>-original.png
    <assertion-id>-target_hidden.png
    <assertion-id>-mask.png
    <assertion-id>-overlay.png
  audio/
    <name>.wav
    <name>.analysis.json
```

Editor-run diagnostics use a project-level bridge directory because they are captured from the Godot Editor plugin rather than a scenario run:

```text
.qa/artifacts/editor-diagnostics/
  latest.json
  <run-id>/
    diagnostics.json
```

`latest.json` and `diagnostics.json` contain `capture_method: "editor_plugin_debugger_errors_panel"`, run metadata, counts, and normalized diagnostic entries with `severity`, `code`, `message`, `script`, `line`, `column`, `source`, and `raw`. `godot-qa diagnostics editor --json` merges this evidence with the command's process/log diagnostics under `.qa/artifacts/diagnostics/<run-id>/`.

Experimental Godot Editor profiler exports use a separate project-level bridge directory because they are captured from the Godot Editor debugger session rather than a runtime scenario:

```text
.qa/artifacts/editor-profiler/
  latest.json
  <run-id>/
    profile.json
```

`latest.json` and `profile.json` contain `source: "editor_script_functions"`, `capture_method: "editor_debugger_raw_servers_profile"`, `experimental: true`, Godot/version/session metadata, bounded retained `frames`, `warnings`, and normalized `script_functions` entries with `signature`, `script`, `function`, `call_count`, `total_ms`, `self_ms`, `internal_ms`, and `max_total_ms`. The editor plugin captures raw debugger messages named `servers:function_signature`, `servers:profile_frame`, and `servers:profile_total`; it does not scrape the visible Profiler table. `godot-qa profile editor export --json` reads this latest artifact, validates the schema, writes a command-owned copy under `.qa/artifacts/profile-editor/<run-id>/profile.json`, and returns the same source under `data.profile`.

Single-viewport runs keep the layout above. S7 layout sweeps add a per-viewport aggregation layer under the run root so each viewport keeps isolated evidence while one command still owns one logical run:

```text
.qa/artifacts/<scenario>/<run-id>/
  meta.json
  request.json
  result.json
  variants/
    1280x720/
      <run-id>/
        meta.json
        request.json
        logs.txt
        result.json
        snapshots/
          default/
            screenshot.png
            tree.json
            ui.json
            focus.json
            errors.json
```

`latest` is a convenience alias; run-id artifact paths are the stable evidence locations. Live/event/telemetry and bundle aliases use symlinks on Unix and NTFS directory junctions on Windows. Junction creation does not require Developer Mode or administrator symlink privileges. Windows junctions contain absolute targets, so recreate them after moving an artifact tree; exported evidence uses relative links instead. Windows replacement has a remove/rename gap and is not atomic. Alias failures must not be mistaken for missing runtime evidence: inspect the owning command's failure or warning and its explicit artifact paths.
Auto-generated run IDs are UTC timestamp-based. If the same scenario is prepared more than once within the same second, later runs get a numeric suffix such as `20260421T120000Z-01` so each run keeps its own artifact root.

In S4, `snapshot` steps write `screenshot.png`, `tree.json`, `ui.json`, `focus.json`, and `errors.json` under deterministic `snapshots/<snapshot-name>/` directories. `logs.txt` remains run-level and is not duplicated per snapshot.
Immediately before snapshot files are written, the runner waits one fresh process frame and forces a draw pass so the JSON artifacts and screenshot come from the same post-step state.
Screenshot-producing scenario runs and live sessions use windowed Godot processes by default so screenshot artifacts come from real viewport pixels. If no host display is available, the CLI may use a safely detected virtual display wrapper for screenshot evidence and records that decision in `launch_display`. If no real display backend is usable for required screenshot evidence, `scenario run` fails early with `type: "display_backend_unavailable"` before runtime execution. If a run root exists, that CLI-owned failure attaches the prepared artifact paths so `meta.json`, `request.json`, `logs.txt`, the intended `result.json`, and `snapshots/` location remain inspectable. If a forced headless run cannot expose viewport pixels, generic snapshot capture may stage a visible diagnostic PNG instead of a transparent placeholder. The Rust CLI still quality-checks every screenshot artifact and treats transparent, blank, or otherwise low-information PNGs as command failures. Visual-pixel assertions opt out of synthetic fallback images and fail unless Godot can provide real rendered viewport pixels.
Filtered snapshots preserve the same snapshot directory names and filenames. Filters affect JSON evidence only; `screenshot.png` is always the full viewport capture, and `errors.json` remains available for runtime diagnostics.

`scenario profile` adds `profile.json` under the same run root:

With `--soak <spec>`, `soak/windows.jsonl` appends one bounded window record at a time and `soak/soak.json` is updated after each window. The latter contains status (`running`, `complete`, `stopped`, or `failed`), `complete`, first failure, worst window, gauge final/high-water values, provenance and engine frame counters. Each window's `engine_frames` and the summary's `engine_frames` count only measured frames. The summary's separate `warmup_engine_frames` counts frames from start through the first measured sample; that sample is the engine-counter boundary, so warmup frames never enter a measured window. A killed process leaves the last flushed partial result. Live soak uses `live/<session-id>/profiles/<soak-run-id>/` with the same two files. These files are measurement evidence; an incomplete run is never a pass.

```text
.qa/artifacts/<scenario>/<run-id>/profile.json
```

The command JSON exposes this path as `data.artifacts.profile`. The profile artifact is run evidence, not a baseline, and it is written alongside `result.json` rather than replacing it.

`profile.json` includes the profiled scenario name, scene, total duration, launch/display metadata, artifact paths, frame timing percentiles, retained monitor samples, retained step timings, opt-in `top_script_methods`, and sample limits. Frame history is bounded by `sample_limits.max_frame_samples`; long scenarios must not write unbounded frame histories by default. Monitor sample history is likewise retained only as a bounded runtime sample set, and optional unsupported Godot monitor counters may be absent while the profile remains valid. Script spans are explicit `qa_profiler.gd` measurements; when project code has not wrapped any functions, `top_script_methods` remains available with an empty `methods` array.

When acceleration is requested, `profile.json.time_control` mirrors the same additive evidence exposed as `data.time_control`: `original`, `requested`, `effective`, `restored`, `restored_matches_original`, `restoration_status`, and `warnings`. Profile timing stays wall-clock evidence; `time_control` explains why gameplay time may advance faster than elapsed host time.

Step timing entries are JSON objects produced by the runner for scene loading, viewport setup, initial frame waits, setup steps, input steps, waits, snapshot capture, assertions, runtime requests, and cleanup/release phases when those phases occur. Entries include duration fields and enough step/assertion context to identify where time was spent without changing scenario pass/fail thresholds.

`session profile` writes a live-session profile artifact under the active session root:

```text
.qa/artifacts/live/<session-id>/profiles/<profile-id>/profile.json
```

The command JSON exposes this path as `data.artifacts.profile`. The artifact includes session id, scene, viewport, sample window, sample limits, bounded frame samples, frame-time percentiles, retained monitor samples, monitor summaries, node process counts, resource activity, and `top_script_methods`. Method-level script profiling is opt-in through `qa_profiler.gd`; uninstrumented projects keep `top_script_methods.status: "available"` with an empty `methods` list. Resource import/file-watch activity may be `{"status":"unavailable"}` unless reload activity is visible in live-session logs.

Live `session profile` writes the profile artifact only after the runtime returns a valid `session.profile` payload and the CLI validates the profile structure. Failed retry or retry-exhaustion paths for `session_bridge_busy`, `session_unreachable`, `session_token_invalid`, `session_connect_timeout`, `profile_result_invalid`, and `artifact_write_failed` must not overwrite previous live-session profile artifacts. Retry metadata such as attempts, bounded backoff timing, final connection status, eligibility, exhaustion state, guidance, and sample window is command JSON evidence; it is not baseline approval and does not change the artifact review contract.

## Portable evidence export

`artifact export <selection.json> --out <new-directory>` creates a review packet separate from the source run roots. Its `evidence.json` and `index.html` use relative paths so the whole directory can move. For each declared source, `sources/<id>/result-summary.json` contains only run identity, allowlisted command/outcome, automatic check status, and failure types. The manifest records revision and environment fields explicitly selected by the caller. It never copies raw `request.json`, `result.json`, `verdict.json`, session/connection metadata, logs, child receipts, arbitrary result data, or failure messages. A missing or malformed source result is retained as an incomplete summary and makes export return nonzero. A failed source remains failed even if its selected screenshots were captured successfully.

Selected original and derived contact-sheet PNGs are labeled separately and copied byte-for-byte; their source and exported SHA-256 values must agree. Metric JSON and result summaries are deliberately projected into allowlisted scalar/structured fields; `source_sha256` identifies the original artifact, `export_sha256` identifies the transformed summary, and `transformation` states the projection. The hashes should not be interpreted as interchangeable. A `visual_review` is `unreviewed` unless an explicit reviewer, method, scale, scope, timestamp, and exact source image SHA-256 were supplied. Automatic check status and visual review status are separate claims. Export never updates screenshot baselines.

Selections require relative artifact paths confined to each chosen run root. Traversal, source symlinks escaping a root, and existing output directories are rejected. See [CLI syntax and selection schema](cli-spec.md#artifact-export) and [a selection example](../examples/evidence-selection.json).

## Check artifacts

`check --tests` writes under the normal check artifact root:

```text
.qa/artifacts/check/<run-id>/
  meta.json
  result.json
  progress.jsonl
  slow-summary.json
  profile.json
  import/
    logs.txt
  tests/
    <test-id>/
      request.json
      logs.txt
      result.json
      artifacts...
```

Isolated mode keeps one Godot launch per selected test and exposes the same per-test artifact directories through each `data.tests[].artifacts` object.

`check --tests --batch` adds a shared batch artifact directory without replacing normal per-test artifact directories:

```text
.qa/artifacts/check/<run-id>/batch/
  request.json
  logs.txt
  summary.json
```

`batch/request.json` records the selected test ids, selection metadata, manifest path, and normalized runtime batch request. Manifest entries with `batch=false` are rejected before this batch directory is produced. `batch/logs.txt` is the shared Godot stdout/stderr stream for the batch process. `batch/summary.json` records `execution_mode: "batch"`, aggregate ok/failure state, selected/skipped counts, per-test status summaries, active-test attribution for runtime errors when known, and contamination diagnostics when practical for held input, scene/script instances, `Engine` settings, and runtime errors between entries.

`progress.jsonl` at `.qa/artifacts/check/<run-id>/progress.jsonl` records append-only progress evidence for selected manifest tests. Each line is one JSON object with the active test `id`, `kind`, `target`, progress state (`started`, `heartbeat`, or `finished`), `started_at`, `elapsed_ms`, and artifact paths for the check root and active test. Batch entries may also identify the shared `batch/logs.txt` stream. Progress evidence must be useful for timeout diagnosis while preserving the project-argument redaction contract; session tokens and secret-looking project args are not written in clear text.

`slow-summary.json` at `.qa/artifacts/check/<run-id>/slow-summary.json` records informational slow-test evidence for `check --tests`. It includes `threshold_ms`, `limit`, `slow_count`, and `slow_tests` sorted by `duration_ms` descending. The default threshold is `1000` ms; manifest and entry-level `slow_summary_threshold_ms` values may lower or raise it for deliberate fixtures such as `slow_manifest_test`. The summary is artifact-backed but non-failing.

`profile.json` at `.qa/artifacts/check/<run-id>/profile.json` is written only for `check --tests --profile`. It is the aggregate profiling artifact for the selected manifest tests and is also referenced by `data.artifacts.profile` plus `data.profile.artifacts.profile` in command JSON. Profile artifacts are per-run files under the check run id and must not overwrite previous check artifacts.

The check profile artifact includes:

- `project_root`, `manifest`, `selection`, `execution_mode`, and the normal check-level `artifacts` map.
- `sample_limits.max_frame_samples`, which bounds retained `frame_times_ms` and `monitor_samples` history.
- `frame_times_ms`, a bounded frame-time sample list, and `frame_time_percentiles_ms` with deterministic percentile summaries.
- `monitor_samples`, the bounded raw Godot `Performance` monitor sample objects, and `monitor_counters`, a per-counter summary such as sample count and latest/min/max/average values when samples are available.
- `unsupported_monitors`, preserving optional monitor names or diagnostic objects that Godot could not provide. Unsupported optional monitors are diagnostic evidence and do not fail an otherwise valid profile.
- `top_script_methods`, an aggregate ranked table of opt-in `qa_profiler.gd` script spans across selected tests.
- `per_test_timings`, including Godot runner `step_timings` when provided and one CLI-added `check_test_total` row for each selected test.
- `tests`, a compact selected-test outcome list with each selected test `id`, `ok`, and `duration_ms`.

Profile command JSON intentionally keeps only a bounded summary under `data.profile`; the artifact is the place to inspect fuller bounded samples and timing rows. Malformed runtime profile payloads fail as `profile_result_invalid` with the active test artifacts attached, and profile artifact write failures fail as `artifact_write_failed` with the intended `profile.json` path. Profiling remains diagnostic-only: failed tests, runtime errors, process failures, and timeouts remain failed validations rather than successful profile runs.

Every selected test still has a normal per-test artifact directory under `.qa/artifacts/check/<run-id>/tests/<test-id>/`. Per-test failures should reference test-local `result.json` evidence first. When stdout/stderr evidence only exists in the shared batch stream, the per-test failure's `artifacts` map references the shared `batch/logs.txt` path and may include line ranges or an active-test marker in structured fields. Shared evidence must supplement per-test artifacts rather than replacing them.

Scenario runs also write `.qa/artifacts/<scenario-name>/<run-id>/progress.jsonl` and `.qa/artifacts/<scenario-name>/<run-id>/slow-summary.json` when progress evidence is available. Scenario `progress.jsonl` records runner-owned step and assertion progress before and after each operation with `step_index` or `assertion_index`, operation kind, redacted safe target/locator context, `started_at`, `elapsed_ms`, and run artifact paths. `slow-summary.json` records `slow_operations` from completed progress heartbeat events, sorted by `elapsed_ms` descending and bounded to ten entries. The default scenario threshold is `1000` ms; a scenario may set `slow_summary_threshold_ms` or `slow_summary.threshold_ms` for fixture-specific evidence such as `slow_step_progress_demo`. Slow scenario summaries do not change pass/fail state.

Timeout cleanup evidence is preserved in the same artifact root rather than requiring process-list inspection. When a timeout kills an owned Godot process, the command JSON and available result artifacts include `cleanup` or equivalent process evidence showing the owned process id when available, kill/termination status, and whether a stale partial `result.json` was ignored. Timeout failures use stable types such as `check_test_timeout` for manifest tests and `scenario_run_timeout` for scenario runs. They preserve partial artifacts, including `meta.json`, `request.json`, `logs.txt`, `progress.jsonl`, `slow-summary.json` when written, and any completed snapshot or per-test directories. Partial artifacts are diagnostic evidence only; a killed Godot process must not be reported as a clean pass because a stale result file exists.

Screenshot-backed visual assertions may add diagnostic images under the run's `diagnostics/` directory or nested assertion-specific diagnostic paths. `original` is the real rendered screenshot used for measurement, `target_hidden` is the comparison capture after a supported target is hidden through runtime evidence hooks, `mask` marks measured target pixels, and `overlay` highlights the measured or failing region for review. Saved-snapshot pixel assertions reuse the snapshot `screenshot.png` as the source image and may attach saved-pixel diagnostics with the same roles when a mask or overlay is produced. These files are evidence under `.qa/artifacts`; they are not screenshot baselines, are not auto-approved, and must not update `qa/baselines`.

Audio scenario steps write under the run root's controlled `audio/` directory:

```text
.qa/artifacts/<scenario>/<run-id>/
  audio/
    <name>.wav
    <name>.analysis.json
```

`<name>.wav` must be a RIFF/WAVE file with nonzero duration, nonzero sample rate, nonzero channel count, and a `data` chunk. `<name>.analysis.json` is written when analysis is requested or available and contains bounded scalar evidence such as `duration_seconds`, `sample_rate`, `channels`, `peak`, `rms`, `silence_ratio`, `near_silence`, `near_silence_ratio`, `clipping_sample_count`, `clipping_region_count`, `loop_boundary_delta`, `transient_count`, `transients_per_second`, `zero_crossing_rate`, `spectral_bands.low|mid|high`, and the timbre facts `spectral_centroid_hz`, `spectral_flatness`, `air_ratio`, `crest_factor`, and `strong_peak_count`. It must not contain unbounded raw sample arrays or large FFT arrays. An optional `<name>.png` waveform + spectrogram (from `audio analyze --spectrogram`) is bounded run evidence, not a baseline.

`result.json` and command JSON may include additive audio fields:

```json
{
  "audio_artifacts": {
    "dodgeball_music": {
      "wav": "/abs/path/.qa/artifacts/.../audio/dodgeball_music.wav",
      "analysis": "/abs/path/.qa/artifacts/.../audio/dodgeball_music.analysis.json"
    }
  },
  "audio_analysis": {
    "dodgeball_music": {
      "duration_seconds": 4.0,
      "sample_rate": 44100,
      "channels": 2,
      "peak": 0.82,
      "rms": 0.18,
      "silence_ratio": 0.0,
      "near_silence": false,
      "clipping_sample_count": 0,
      "loop_boundary_delta": 0.03,
      "transient_count": 42,
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
      "strong_peak_count": 8
    }
  }
}
```

Audio artifacts are run evidence, not baselines. They are never auto-approved and should be inspected manually when subjective audio quality matters.

Live audio capture, export, and render use the live session artifact root instead of a scenario run root:

```text
.qa/artifacts/live/<session-id>/audio/
  <name>.wav
  <name>.analysis.json
```

`.qa/artifacts/live/latest` may point at the current or most recent live session for CLI convenience, but JSON should report the resolved session path so evidence remains stable after later sessions. Manual output paths from `godot-qa audio capture --out ...` are CLI-side copies from the controlled live artifact. The Godot runtime receives only the controlled `audio_dir` and intended `wav_path` under the session root.

Live `.analysis.json` files reuse the S64 scenario audio analysis fields and add bounded time-window summaries for short exploratory captures:

```json
{
  "duration_seconds": 3.0,
  "sample_rate": 48000,
  "channels": 2,
  "peak": 0.95,
  "rms": 0.21,
  "silence_ratio": 0.1,
  "near_silence_ratio": 0.12,
  "clipping_sample_count": 12,
  "clipping_region_count": 1,
  "transient_count": 18,
  "transients_per_second": 6.0,
  "zero_crossing_rate": 0.14,
  "spectral_bands": {
    "low": 0.35,
    "mid": 0.41,
    "high": 0.24
  },
  "spectral_centroid_hz": 2100.0,
  "spectral_flatness": 0.38,
  "air_ratio": 0.27,
  "crest_factor": 4.2,
  "strong_peak_count": 6,
  "window_seconds": 1.0,
  "hop_seconds": 0.5,
  "peak_by_window": [],
  "rms_by_window": [],
  "transients_by_window": [],
  "suspected_event_spam_regions": [],
  "clipping_regions": [],
  "silence_regions": [],
  "loop_boundary_delta": 0.03
}
```

The default window bounds are `window_seconds = 1.0` and `hop_seconds = 0.5`; any configurable values must be finite, positive, bounded, and written to the analysis file. Window arrays and region lists are summaries only. Artifact JSON and command JSON must not embed raw sample arrays.

Live sessions use a parallel layout under `.qa/artifacts/live/<session-id>/`:

```text
.qa/artifacts/live/<session-id>/
  meta.json
  session.json
  request.json
  logs.txt
  screenshots/
    <capture-name>.png
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

`session.json` is a recoverable copy of the live process/session metadata also written to `.qa/session.json`. The token value for that session is not stored here. It lives only in `.qa/session.token`, while both session metadata files record the token file path for authenticated live commands.
On Windows, saved session metadata also includes optional `process_identity`, an opaque unsigned 64-bit process-creation timestamp (Windows FILETIME). The same field is stored for the sidecar in `.qa/debug_host.json`. Unix records omit it, and older records without it remain readable. Windows process checks compare the saved creation identity as well as the PID: a reused PID is stale and must never be terminated as the original session. Legacy records without identity cannot authorize fallback process termination; an authenticated bridge stop can still succeed, otherwise restart the session to establish current metadata.

Live screenshot capture stages PNGs under `screenshots/` inside the controlled session artifact root. `godot-qa screenshot capture --out ...` then copies the staged PNG to the requested destination on the CLI side instead of letting Godot write arbitrary output paths.

Screenshot sequences write review artifacts under the caller's `--out` directory:

```text
<sequence-dir>/
  manifest.json
  frames/
    frame_0000.png
    frame_0015.png
    frame_0030.png
  contact_sheet.png
  contact_sheet.json
```

`manifest.json` has `command: "screenshot.sequence"`, optional `scenario`, `scenario_path`, and `scenario_scene`, `scenario_steps_replayed: false`, optional `state_guard_requirements`, `timing_mode`, `frame_count_requested`, `capture_every_frames`, optional `seconds` and `fps`, `captured_count`, and `frames`. Each frame entry records `index`, `engine_frame`, `time_msec`, relative `path`, `width`, `height`, optional `state_guards`, and `quality`. `quality` includes `low_information`, `low_information_reasons`, `pixel_count`, `non_background_pixel_count`, `non_background_ratio`, and `visual_risk_warnings`.

Command JSON for `screenshot sequence` reports the same bounded timing summary plus `low_information_frame_count`, `failed_capture_count`, `visual_risk_warning_count`, `first_failed_frame`, and artifact paths. Low-information frame failures keep `type: "screenshot_low_information"` unless `--allow-low-information` is explicitly supplied. Visual-risk findings remain warnings unless strict mode is requested, and they do not judge subjective art quality.

Contact-sheet generation reads the sequence manifest in order, preserves image aspect ratio in thumbnails, and places frame/time labels in a non-game artifact margin. The companion JSON records `source_images`, grid geometry, `columns`, `rows`, `thumb_width`, derived thumbnail sizes, label metadata, output path, and manifest path. Missing, unreadable, or dimensionally unsupported source images are artifact failures rather than skipped frames. Sequence frames, contact sheets, diffs, and companion JSON are review evidence only; baseline approval remains manual and these commands must not update `qa/baselines`.

Review-template artifacts are normal scenario or live evidence. `review init` only writes a reusable scenario or live-script file; when callers later run it with `scenario run --file` or `session run-script`, the resulting screenshots, screenshot sequences, contact sheets, traces, physics collision reports, audio captures, logs, errors, and runtime result JSON use the same artifact contracts as the underlying commands. Template-generated evidence never auto-approves baselines, never writes to `qa/baselines`, and never replaces final human review.

Visual audit reports are machine-readable review evidence under `data.visual_audit` and, when persisted, in a `visual_audit.json` artifact. They are not screenshot baselines, are not auto-approved, and do not judge subjective art quality. The report object records `screenshot_path`, `width`, `height`, `targets`, `findings`, `thresholds`, `strict`, and `artifacts`.

Each `targets[]` entry records measurable facts only: `target_id`, `screenshot_path`, `inside_viewport`, `pixel_area`, `bounding_box_width`, `bounding_box_height`, optional `visible_pixel_count`, `estimated_local_background_color`, `estimated_target_color_or_median`, `relative_luminance_target`, `relative_luminance_background`, `contrast_ratio_estimate`, `local_color_distance`, `edge_contrast_estimate`, `small_target_warning`, `low_contrast_warning`, `offscreen_warning`, optional `occlusion_or_missing_pixels_warning`, `target_rect`, and `clamped_rect`. `thresholds` records the effective `min_pixel_area`, `min_width`, `min_height`, `min_contrast_ratio`, `min_color_distance`, and `min_edge_contrast` values so reviewers can tell which defaults or overrides produced a warning.

Sequence visual audits add aggregate fields: `frames_checked`, `frames_with_target_missing`, `minimum_pixel_area_across_frames`, `minimum_contrast_across_frames`, and `worst_frame_path_by_rule`. These fields summarize the reviewed sequence without embedding frame pixels or approving any baseline.

Visual audit warning objects use stable IDs under `rule_id`: `visual_audit.target_missing`, `visual_audit.target_offscreen`, `visual_audit.target_too_small`, `visual_audit.low_luminance_contrast`, `visual_audit.low_color_distance`, `visual_audit.low_edge_contrast`, `visual_audit.target_bounds_unavailable`, `visual_audit.sequence_target_missing_frames`, and `visual_audit.annotated_artifact_failed`. Findings remain warning severity unless `--strict-warnings` was requested, in which case the same findings become failures in command JSON.

When `--annotate-out <path.png>` is supplied, the artifact map includes `annotated` pointing at the annotation PNG. Annotation PNG artifacts draw target boxes and warning evidence onto the output artifact only; they do not mutate the running game, update source screenshots, or write to `qa/baselines`.

Visual isolate writes an output directory with `manifest.json`, `frames/`, `diffs/`, and a contact-sheet PNG/JSON. The manifest records `command: "visual.isolate"`, the project provider, fixture driver/action, setup script metadata when supplied, provider state, selected layers, frame entries for baseline/layer-only/layer-disabled/diff images, per-layer `changed_pixel_bounds`, `changed_pixel_ratio`, and `straight_edge_or_rectangular_region_warning`. These artifacts are ownership diagnostics only; they never update baselines and never add visible debug UI to the running project.

Live physics inspection writes `.qa/artifacts/live/<session-id>/physics/<capture-id>/physics.json` and reports that path as `data.artifacts.physics`; optional `--snapshot` evidence uses the normal `snapshots/<snapshot-name>/` bundle.
Scenario `physics_overlay` steps preserve collector evidence in `result.json.physics_overlays`, keyed by step index/name or target. Each entry records the target, viewport flag, optional snapshot name, and raw overlay metadata from the same runtime collector used by live `physics overlay`. Snapshot review uses the saved `snapshots/<snapshot-name>/screenshot.png` and this metadata; missing screenshot or metadata is an artifact failure and does not trigger a fresh runtime capture. When review mode is given only a scenario name, `.qa/artifacts/<scenario>/latest` is preferred; otherwise the lexicographically newest run directory containing `result.json` is selected deterministically.

Scenario `record_state` steps preserve read-only state evidence in `result.json.state_records`, an object keyed by each step's `name`. Each entry records `step_index`, `name`, the source descriptor (`source` plus `provider`, or `target`/`resolved`/`method`), the optional `path`, `present`, the bounded `value`, and `value_bytes`. The value is a single controlled read (JSON-compatible, byte-capped runtime-side), so the map stays bounded. `scenario run` lifts `state_records` into the command JSON `data` alongside `node_inspections` and `policy_drives`, and `scenario sweep` echoes each child run's `state_records` in its `per_seed[]` entries so balance metrics across a seed range are a single read of `sweep.json`.

Physics collider overlay rendering writes three sibling files derived from the requested `--out` PNG:

- `<stem>.base.png`: the real runtime or saved-snapshot screenshot used as the background.
- `<stem>.png`: the rendered overlay image requested by `--out`.
- `<stem>.overlay.json`: machine-readable metadata for the rendered overlay.

For example, `--out .qa/artifacts/manual/physics_overlay_actor.png` produces `.qa/artifacts/manual/physics_overlay_actor.base.png`, `.qa/artifacts/manual/physics_overlay_actor.png`, and `.qa/artifacts/manual/physics_overlay_actor.overlay.json`. Live commands may write these wherever `--out` points, commonly under `.qa/artifacts/manual/`. Scenario review commands write the same sibling set from saved scenario evidence. Scenario runs themselves keep collector evidence inside `.qa/artifacts/<scenario>/<run-id>/result.json.physics_overlays`; those entries are not overlay PNGs until `godot-qa physics overlay --scenario <name> --snapshot <name> --out <png>` renders them.

The `.overlay.json` file has this stable shape:

```json
{
  "command": "physics.overlay",
  "target": "qa:overlay_actor",
  "viewport": false,
  "legend": {
    "body_collision": {"color": "#ff00c8", "style": "solid outline"},
    "area_collision": {"color": "#00e1ff", "style": "solid outline"},
    "aabb": {"color": "#ffdc00", "style": "thin rectangle"},
    "contact_point": {"color": "#ff2828", "style": "dot"},
    "contact_normal": {"color": "#ffffff", "style": "line"},
    "visual_footprint": {"color": "#00eb5a", "style": "solid outline"}
  },
  "render": {
    "rendered": {
      "shape_count": 3,
      "contact_count": 0,
      "visual_footprint_count": 1
    }
  },
  "artifacts": {
    "screenshot": "/abs/path/physics_overlay_actor.base.png",
    "overlay": "/abs/path/physics_overlay_actor.png",
    "metadata": "/abs/path/physics_overlay_actor.overlay.json"
  },
  "shapes": [],
  "contacts": [],
  "project_state": {},
  "runtime_metadata": {
    "shape_count": 3,
    "body_count": 2,
    "area_count": 1,
    "contact_count": 0,
    "warnings": []
  },
  "warnings": []
}
```

Each `shapes` entry is Godot-owned runtime evidence and may include `logical_id`, `qa_id`, `node_path`, `node_type`, `shape_type`, `layer`, `mask`, `world_aabb`, `screen_aabb`, `screen_points`, and booleans such as `is_area`, `is_body`, and `is_soft`. The renderer draws only from this runtime/saved metadata and optional approved project visual-footprint state; it must not infer a visual footprint from arbitrary sprites. Summary counts in CLI JSON come from the same metadata: `shape_count`, `body_count`, `area_count`, `contact_count`, `visual_footprint_count`, and `warnings`.

Overlay artifacts are evidence for review and debugging. They are not screenshot baselines, are not auto-approved, do not update `qa/baselines`, and do not represent a game-facing debug HUD.

Live runtime trace recording writes `.qa/artifacts/live/<session-id>/traces/<trace-id>/trace.json` and reports that path as `data.artifacts.trace`; optional start/end snapshots use the normal `snapshots/trace-start/` and `snapshots/trace-end/` bundles.

Event trace artifacts write the project-provided event stream and derived summary under:

```text
.qa/artifacts/live/<session-id>/events/<event-trace-id>/
  events.json
  summary.json
```

For convenience, the live artifact root may maintain `.qa/artifacts/live/latest/events/latest/events.json` and `.qa/artifacts/live/latest/events/latest/summary.json` pointers or equivalent latest-path behavior. Latest paths are convenience aliases only; the stable evidence path is the session/event-trace ID directory reported in command JSON.

The exact reported event artifact paths are `.qa/artifacts/live/<session-id>/events/<event-trace-id>/events.json` and `.qa/artifacts/live/<session-id>/events/<event-trace-id>/summary.json`.

`events.json` contains command metadata, target, provider, sample bounds, optional pairing/rate-limit config references, an `events` array, warning objects, and artifact paths. Each event object must be JSON-compatible and should include:

```json
{
  "event_id": "unique-or-stable-id",
  "frame": 123,
  "time_msec": 2048,
  "category": "gameplay|vfx|sfx|ui|collision|custom",
  "event_type": "string_event_type",
  "logical_id": "optional_actor_or_object_id",
  "position": {"x": 0, "y": 0},
  "payload": {}
}
```

`event_id`, `frame`, `time_msec`, `category`, and `event_type` are the required event fields for reliable summaries. `logical_id`, `position`, and `payload` are optional. Payloads are preserved as project-provided JSON-compatible data within bounded size limits; unbounded or non-JSON-compatible payloads are rejected or truncated with `event_trace.event_not_json_compatible` or `event_trace.event_stream_truncated`. If `event_id` is missing, the CLI deduplicates cautiously from frame/category/event_type/logical_id/payload hash and reports `event_trace.event_missing_id`.

`summary.json` contains the same summary object reported by `event trace --json` and `event summarize --json`: `total_event_count`, `event_count_by_category`, `event_count_by_type`, `event_rate_per_second_by_type`, `max_events_in_one_frame`, `duplicate_event_count`, `missing_event_id_count`, `event_spam_warnings`, `paired_event_warnings`, `offscreen_event_warnings`, and `provider_error_count`.

Warning objects contain at least `rule_id`, `severity`, and `message`, and should include `target`, `provider`, expected threshold or pairing rule, actual measured value, source frame/event IDs, and artifact paths when available. Required event trace warning IDs are `event_trace.provider_missing`, `event_trace.provider_invalid`, `event_trace.event_missing_id`, `event_trace.event_not_json_compatible`, `event_trace.event_stream_truncated`, `event_trace.event_spam_risk`, `event_trace.paired_event_missing`, `event_trace.offscreen_event_risk`, `event_trace.provider_error`, and `event_trace.no_events_recorded`.

Live `physics collision-report` reports write `.qa/artifacts/live/<session-id>/collision-reports/<report-id>/collision_report.json` and report that path as `data.artifacts.report`. Live collection also links the source trace artifact under `data.artifacts.trace`; trace-based summarization keeps the original `.qa/artifacts/live/<session-id>/traces/<trace-id>/trace.json` path instead of rerunning Godot. These artifacts are numeric evidence for review and debugging, not screenshot baselines or automatic game-feel approval.

`collision_report.json` contains the command, target when present, source trace path when present, bounded sample config, `summary`, `metrics`, and warning objects. Metric keys include `frames_sampled`, `contact_frame_count`, `contact_count_by_surface_type`, `unique_contact_ids`, `same_contact_max_consecutive_frames`, `largest_overlap_depth`, `max_speed_before_contact`, `max_speed_after_contact`, `max_speed_delta_after_contact`, `normal_velocity_before`, `normal_velocity_after`, `normal_velocity_retention_ratio`, `tangent_velocity_retention_ratio`, `velocity_sign_reversal_count`, `max_angular_velocity_before_contact`, `max_angular_velocity_after_contact`, `angular_velocity_spike_count`, `soft_contact_duration_frames`, `hard_contact_duration_frames`, `rescue_push_count`, `stuck_like_frame_count`, and `low_progress_while_contact_frames`.

Warnings are objects with at least `rule_id` and `severity`, and should include target, expected threshold, actual measured value, source frame/contact evidence, and artifact paths when available. Provider-specific fields that require explicit project classification, such as soft contact duration or `collision_report.soft_contact_behaves_hard`, are stored as `not_evaluated` when provider evidence is missing or invalid. Missing optional provider state may produce `collision_report.provider_missing`; invalid provider payloads may produce `collision_report.provider_invalid`.

## Surface audit artifacts

`surface audit` writes a machine-readable report artifact to the requested `--out` path or to a controlled default path under `.qa/artifacts/.../surface_audit.json`. Optional heatmaps are written only when `--heatmap-out <path.png>` is supplied.

```text
.qa/artifacts/manual/surface_audit.json
.qa/artifacts/manual/surface_corridor_heatmap.png
```

`surface_audit.json` contains the command, normalized request, provider response, metrics, warnings, and artifact paths:

```json
{
  "command": "surface.audit",
  "request": {
    "provider": "qa:surface_provider",
    "provider_method": "qa_surface_probe",
    "mode": "route",
    "route": ["qa:route_start", "qa:route_mid", "qa:route_finish"],
    "sample_step": 16.0,
    "actor_radius": 24.0,
    "min_corridor_width": 72.0
  },
  "provider_response": {
    "command": "surface.audit_probe",
    "samples": []
  },
  "metrics": {
    "sample_count": 0,
    "walkable_count": 0,
    "blocked_count": 0,
    "soft_count": 0,
    "unknown_count": 0,
    "spawn_clearance_min_distance": null,
    "spawn_clearance_pass": null,
    "minimum_corridor_width": null,
    "corridor_width_pass": null,
    "hard_block_cluster_count": 0,
    "soft_block_cluster_count": 0,
    "dead_end_pocket_count": 0,
    "largest_dead_end_area": 0.0,
    "route_clearance_fail_points": [],
    "symmetry_delta_score": null,
    "left_right_passability_delta": null,
    "provider_missing_or_invalid": true
  },
  "warnings": [],
  "artifacts": {
    "surface_audit": "/abs/path/.qa/artifacts/manual/surface_audit.json",
    "heatmap": "/abs/path/.qa/artifacts/manual/surface_corridor_heatmap.png"
  }
}
```

Heatmap artifacts are image evidence, not runtime overlays. Walkable samples are rendered green or blue, soft samples yellow or orange, blocked samples red or dark, unknown samples purple or gray, route/anchor samples visible markers, and actor-radius clearance circles may be drawn as outlines. Reports and heatmaps are not baselines, are not auto-approved, and must not update `qa/baselines`; baseline review remains a separate human action.

## `meta.json`

`meta.json.command` records a shell-ready `godot-qa scenario run ...` invocation for that artifact set, including non-default CLI overrides such as `--project`, `--godot-bin`, and `--artifact-dir`. `meta.json.godot_args` records effective Godot engine arguments passed before Godot's `--` separator, including config defaults followed by scenario-specific args. `meta.json.project_args` records effective project/game arguments passed after the separator, including config defaults followed by scenario-specific args, with secret-looking values redacted for persisted metadata. `meta.json.time_control` records the requested acceleration configuration when present; it is request metadata, not proof that the runtime applied or restored Engine settings.

`meta.json.assertion_manifest` retains the expected runtime assertions from the original normalized scenario, before CLI-only assertions are removed. It contains `schema_version: 1`, `entries`, and `cli_assertions_excluded`. Original JSON-pointer `source_id` values remain stable when filtering changes the corresponding `runtime_path`; for example, `/steps/2/assert/1` can execute at `/steps/2/assert/0`. Final assertions use `/assert/<index>`. Repeated equal assertions have distinct source IDs. Every viewport variant keeps these original locations while using its own run nonce.

```json
{
  "tool": "godot-qa",
  "tool_version": "0.0.0",
  "project_root": "/path/to/project",
  "scenario": "main_menu_start_opens_pause",
  "scene": "res://ui/main_menu.tscn",
  "viewport": [1920, 1080],
  "started_at": "2026-04-21T12:00:00Z",
  "command": "godot-qa scenario run main_menu_start_opens_pause --json",
  "godot_args": ["--single-window"],
  "project_args": ["--save-slot", "qa", "--token", "<redacted:6>", "API_KEY=<redacted:11>"],
  "scenario_path": "/path/to/project/qa/scenarios/examples/main_menu_start_opens_pause.json"
}
```

The redaction contract preserves argument count, order, and enough shape to debug command construction. Values following secret-bearing keys such as `--token`, `--password`, or `--client-secret` are stored as `<redacted:N>`, and secret-looking `KEY=VALUE` entries are stored as `KEY=<redacted:N>`, where `N` is the original value character count. Non-secret project args remain unchanged.

## `request.json`

`request.json.run.godot_args` and `request.json.run.project_args` mirror the effective run-level launch metadata, with the same persisted redaction applied to `run.project_args`. `request.json.scenario.godot_args` and `request.json.scenario.project_args` preserve the normalized effective scenario payload sent to the Godot runner, including raw config and scenario project arg values required for execution. `request.json.run.time_control` and `request.json.scenario.time_control` preserve requested acceleration settings when present, and the internal runtime request may also include `time_control_warnings` computed by the CLI. Consumers that persist, summarize, or publish request metadata should use the redacted run-level fields for display and treat raw scenario-level `project_args` as sensitive runtime input.

New scenario requests include top-level `assertion_proof`, matching the retained assertion manifest. This marker requires the addon to verify that every runtime assertion matches its requested source entry before loading the scene. The internal stripped runtime request carries the same manifest. The new addon continues accepting older requests without this marker; new CLI executions require proof and fail with `assertion_proof_missing` and addon-update guidance when an older addon omits it.

## `result.json`

```json
{
  "ok": false,
  "scenario": "settings_menu_layout",
  "scene": "res://ui/settings_menu.tscn",
  "godot_args": ["--single-window"],
  "project_args": ["--save-slot", "qa"],
  "duration_ms": 1450,
  "snapshots": ["before", "after_start"],
  "artifacts": {
    "run_root": "/path/to/project/.qa/artifacts/settings_menu_layout/20260421T120000Z",
    "result_path": "/path/to/project/.qa/artifacts/settings_menu_layout/20260421T120000Z/result.json",
    "logs_path": "/path/to/project/.qa/artifacts/settings_menu_layout/20260421T120000Z/logs.txt",
    "screenshot_paths": ["/path/to/project/.qa/artifacts/settings_menu_layout/20260421T120000Z/snapshots/before/screenshot.png"],
    "diagnostic_paths": ["/path/to/project/.qa/artifacts/settings_menu_layout/20260421T120000Z/snapshots"]
  },
  "snapshot_artifacts": {
    "before": {
      "root": "/path/to/project/.qa/artifacts/settings_menu_layout/20260421T120000Z/snapshots/before",
      "screenshot": "/path/to/project/.qa/artifacts/settings_menu_layout/20260421T120000Z/snapshots/before/screenshot.png",
      "tree": "/path/to/project/.qa/artifacts/settings_menu_layout/20260421T120000Z/snapshots/before/tree.json",
      "ui": "/path/to/project/.qa/artifacts/settings_menu_layout/20260421T120000Z/snapshots/before/ui.json",
      "focus": "/path/to/project/.qa/artifacts/settings_menu_layout/20260421T120000Z/snapshots/before/focus.json",
      "errors": "/path/to/project/.qa/artifacts/settings_menu_layout/20260421T120000Z/snapshots/before/errors.json"
    }
  },
  "screenshot_quality": {
    "before": {
      "path": "/path/to/project/.qa/artifacts/settings_menu_layout/20260421T120000Z/snapshots/before/screenshot.png",
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
    "display_available": false,
    "wayland_available": false,
    "wayland_requested": false,
    "virtual_display": {
      "used": true,
      "wrapper": "xvfb-run",
      "command_prefix": ["xvfb-run", "-a"],
      "reason": "screenshot evidence requires rendered pixels and no host display was detected"
    }
  },
  "failures": [
    {
      "type": "outside_viewport",
      "target": "qa:apply_button",
      "viewport": [1280, 720],
      "rect": [1090, 690, 240, 64],
      "severity": "error"
    }
  ]
}
```

`artifacts` remains the flat run-level path map owned by the CLI handoff: `run_root`, `request_path`, `result_path`, `logs_path`, `screenshot_paths`, and `diagnostic_paths`.

Scenario results also carry an always-on advisory `timing` object — coarse engine-side phase spans in fractional milliseconds: `scene_load_ms` (PackedScene load + instantiate + `add_child`, i.e. the game's `_ready`), `first_frame_ms`, `steps_ms`, `asserts_ms`, `teardown_ms` (scene free + settle frame), and `engine_total_ms` (mirrors `duration_ms`). It exists for performance attribution (where a scenario's wall clock goes), is independent of the `scenario profile` gate, is never validated or compared for equivalence, and its exact key set may evolve.

Cold scenario command summaries add `cold_timing`: `artifact_prepare_ms` (nullable
when the caller prepared artifacts outside the measured path),
`request_prepare_ms` (launch preflight/request preparation), and
`cli_finalize_ms` (result validation and CLI post-processing, when completed).
These are monotonic millisecond durations, separate from Godot's `timing`.

`data.process.termination` records the native status of the CLI's directly
launched child: `scope: "direct_child"`, nullable `exit_code`, nullable Unix
`signal` and `core_dumped`, platform-native `native_status`, and `tool_initiated`.
With a launch prefix this describes the wrapper, which may hide its Godot child's
status. Existing `data.process.exit_code` remains compatible, including `-1`
for signals/timeouts. Unknown or synthetic process evidence is null.
`import --json` exposes the same `data.process` evidence, and failed imports
include `termination` alongside their existing scalar failure `actual`.

`data.process.timing` contains monotonic millisecond observations: `spawn_ms`
(process preparation through spawn), nullable `runtime_started_observed_ms`
and `result_observed_ms` (offsets from process preparation start), nullable
`post_result_exit_ms` (from observing a matching result until observed child exit),
`cleanup_ms`, and `total_ms`. Runtime-start observations require the nonce-bound
progress marker; missing observations stay null. `total_ms` includes captured
output collection. These overlapping spans are diagnostic and do not sum to
scenario wall time: launcher/project-lock waits, wrapper work, and final artifact
writes may fall outside individual spans. They do not change deadlines, verdicts,
or cold/warm equivalence.

For example, a directly launched child terminated by Unix signal 15 can report:

```json
{
  "exit_code": -1,
  "termination": {
    "scope": "direct_child",
    "exit_code": null,
    "signal": 15,
    "core_dumped": false,
    "native_status": "signal: 15 (SIGTERM)",
    "tool_initiated": false
  }
}
```

The platform formats `native_status`; consumers should use the structured signal
and exit fields. A signal identifies termination without establishing its cause.
`godot_args` and `project_args` remain separate top-level arrays in runtime `result.json`; runtime results report the effective values seen by the Godot runner. Persisted command summaries and bundle metadata apply redaction to secret-looking `project_args`, but the runtime result remains the Godot-owned execution payload.
`frame_pacing` is optional CLI-owned metadata in command JSON and persisted `result.json` for fastest automated runs. It contains `mode`, `fixed_fps`, `source`, `project_setting`, and `generated_godot_args`. It documents launch pacing such as generated `--fixed-fps` and `--disable-vsync`; it is not runtime Engine state.
`time_control` is optional additive runtime evidence in `result.json` and command JSON. When present, it must be an object with the field names `original`, `requested`, `effective`, `restored`, `restored_matches_original`, `restoration_status`, and `warnings`. These fields document the Godot-owned Engine state transition around `Engine.time_scale`, `Engine.physics_ticks_per_second`, and `Engine.max_physics_steps_per_frame`; `restoration_status` should be `restored` when `restored` matches `original`.
`snapshot_artifacts` is a separate top-level object keyed by snapshot name. Each snapshot entry must itself be an object of string paths, typically including `root`, `screenshot`, `tree`, `ui`, `focus`, and `errors`.
`screenshot_quality` is a CLI-owned object keyed by snapshot name when a scenario saves screenshots. It records dimensions, exact dominant RGBA colors, non-background pixel counts, and `low_information` reasons. Low-information screenshots fail by default with `type: "screenshot_low_information"`; `snapshots.fail_on_low_information` remains accepted only for compatibility.
Screenshot quality, saved-pixel assertions, and comparisons share analysis within
one finalization. The cache retains at most two decoded images; a comparison can
also decode its baseline. No image cache survives the run. This optimization
preserves evidence, thresholds, original PNGs,
comparison metrics, and required diff artifacts.
`visual_risk` is a CLI-owned object present only when `scenario run` is invoked with `--visual-risk` or `--visual-risk-strict`. It contains `strict` and `findings`. Each finding uses a stable `visual-risk.*` rule ID, warning severity unless strict mode converts it to an error, measured facts under `actual`, and artifact path references under `artifacts` when available. Current rule IDs are `visual-risk.low_visual_diversity`, `visual-risk.huge_flat_shape`, `visual-risk.debug_primary_colors`, and `visual-risk.missing_visible_target_pixels`.
Visual-risk measured facts are specific to the rule. Screenshot-wide findings can include `width`, `height`, `pixel_count`, `unique_color_count`, `dominant_color_ratio`, `largest_flat_region_pixel_count`, `largest_flat_region_ratio`, `largest_flat_region_rgba`, `debug_primary_pixel_count`, and `debug_primary_ratio`. Missing-target-pixel findings can include `target`, `snapshot`, `screenshot`, `target_rect`, `visible_pixel_count`, `visible_pixel_ratio`, `useful_pixel_count`, and `useful_pixel_ratio`.
Visual-risk findings are screenshot artifact-backed risk signals only. They do not duplicate screenshot files, judge art quality, approve or reject baselines, or replace screenshot comparison and visual assertions. They are also distinct from `screenshot_quality`: screenshots classified as low-information still fail as `screenshot_low_information` and are not reclassified as `visual-risk.*`.
Visual assertions may add `visual_evidence` objects to assertion outcomes and failures. These additive fields describe the source screenshot path, the measured viewport rect or mask, useful-pixel count and ratio, the configured thresholds (`min_count` and/or `min_ratio`), and the target locator. Because target rects are measured in the 2D canvas space while screenshots are captured at the OS window resolution, live `visible_pixels` evidence also carries the additive `target_rect_screenshot` (the canvas rect converted into screenshot space and actually sampled), `canvas_to_screenshot` (`{scale: [sx, sy], offset: [ox, oy]}`), and `screenshot_size`; these equal `target_rect`, an identity transform, and the screenshot dimensions when the window matches the canvas. An unforeseen capture/scaling mismatch (the transformed full canvas not matching the screenshot size) fails loudly with `screenshot_geometry_mismatch`. Saved-snapshot pixel failures use the same artifact map as the selected snapshot, including `root`, `run_root`, `screenshot`, `tree`, `ui`, `focus`, and `errors` when present, and may add diagnostic paths such as `mask`, `overlay`, or `visual_mask`. Consumers must treat these keys as optional additive evidence: existing full snapshot bundles and filtered snapshot bundles keep the same filenames, and filtering state remains inside the JSON artifacts rather than in artifact names.
Saved-pixel `visual_evidence` is measured from real screenshot pixels inside the target viewport rect or generated mask. It is not derived from scene-tree `visible` state, node origin, or a placeholder image. Typical fields include `target`, `source_screenshot` or `screenshot`, `target_rect` or `rect`, `useful_pixel_count`, `useful_pixel_ratio`, `min_count`, `min_ratio`, `low_information`, and `low_information_reasons`. Saved snapshot node `visual_evidence` records `rect_screenshot` (screenshot-space) alongside the canvas-space `rect`; saved-pixel measurement prefers `rect_screenshot` when present and falls back to `rect` so snapshots captured before this field keep working.
`launch_display` is CLI-owned metadata copied into the command JSON `data` and runtime `result.json` when a Godot launch is prepared. It records the selected headless/windowed mode, display driver, host display availability, Wayland selection facts, and virtual-display wrapper metadata when one was selected.
Display selection failures are early CLI-owned failures. A `display_backend_unavailable` failure means screenshot-backed evidence required real rendered pixels, no usable host display was detected, and no available virtual display wrapper was safely selected. When artifact preparation already succeeded, `failure.artifacts` carries the same run-level path map as `data.artifacts`.
Filtered snapshot state is recorded inside each JSON artifact that supports filtering, not by renaming files. Consumers should inspect `tree.json.filter`, `ui.json.filter`, or `focus.json.filter` when they need to know whether evidence is scoped, depth-limited, QA-only, or metadata-limited.

`scenario run` command JSON treats the runtime `result.json` as Godot-owned runtime truth and layers CLI-owned summaries around it. `data.artifacts` is the prepared run-level path map (`run_root`, `request_path`, `result_path`, `logs_path`, `screenshot_paths`, and `diagnostic_paths`) with validated runtime-reported artifact keys overlaid; the CLI does not drop prepared paths when the runtime reports only a subset. `data.runtime_result` summarizes whether the runtime payload at the prepared `result_path` was present and structurally valid, and whether it reported `ok` plus runtime failures. `data.process` summarizes the Godot process exit independently. `data.logs` summarizes log health independently. Its shape is `path`, `status`, `error_count`, `warning_count`, `ignored_count`, and `ignore_patterns_applied`: the last two record how many engine `ERROR:`/`WARNING:` lines an ignore list silenced during that scan and how many project `logs.ignore` patterns were in force, and both are `0` when log health is reported without scanning (timeout, missing or invalid runtime `result.json`). For a warm (pooled-session) run that `logs.txt` is not a whole process log but a slice of the pooled session's `logs.txt`, cut out between the `[godot-qa] scenario-log begin|end <run-id>` stderr markers the addon writes around each warm scenario and written to `<run_root>/logs.txt` before the run is finalized, so log health is computed exactly as it is for cold; the session-level `logs.txt` under `.qa/artifacts/live/<session-id>/` stays intact and is never truncated, rotated or reopened by the CLI, and `data.warm.log_slice` records which byte window was taken. `data.outcome` is the final CLI decision after combining process, runtime, and log state.

Missing and malformed runtime payloads are CLI-owned artifact failures, not runtime assertion failures. If `result.json` is absent, `scenario run` fails with `scenario_result_missing`, includes the prepared result and log paths in `data.artifacts` and `failure.artifacts`, and sets `data.runtime_result.status` to `missing`. If `result.json` cannot be parsed or fails the structural contract below, `scenario run` fails with `scenario_result_invalid` and sets `data.runtime_result.status` to `invalid`. Runtime assertion failures use `data.runtime_result.status: "failed"` with the runtime failure count, while clean runtime payloads use `status: "passed"` even if the separate process summary later causes a strict `godot_run_failed` outcome. A wrapper-only pass-with-warning still leaves the non-zero process status visible under `data.process.warnings` and does not create a top-level failure contradiction.

## Scenario and bundle artifacts

Scenario runtime results include `assertion_coverage`; finalized command results expose it under `data.assertion_coverage`. Schema 1 carries `run_id`, `nonce`, `expected`, `started`, `finished`, `passed`, `failed`, `complete`, `cli_assertions_excluded`, and ordered `entries`. Each entry contains original `source_id`, filtered `runtime_path`, `operation`, diagnostic `assertion`, lossless `assertion_identity`, and `status` (`not_started`, `running`, `passed`, or `failed`). For example:

```json
{
  "source_id": "/steps/2/assert/1",
  "runtime_path": "/steps/2/assert/0",
  "operation": "exists",
  "assertion": {"exists": "qa:menu"},
  "assertion_identity": ["object", {"exists": ["string", "qa:menu"]}],
  "status": "passed"
}
```

The `json_f64_le_v1` identity uses typed nodes: `["null"]`, `["bool", value]`, `["string", value]`, `["number", binary64_hex]`, `["array", items]`, and `["object", fields]`. Numeric identities contain 16 lowercase hexadecimal characters encoding little-endian binary64; signed zero is normalized. This avoids precision loss from Godot's JSON number formatting and prevents user arrays or dictionaries from colliding with internal tags. The diagnostic `assertion` echo can have rounded numbers and is not the binding. Integers that cannot round-trip exactly through binary64 and values whose precision changes during Godot JSON parsing are rejected before scene execution. Android reverses controlled path mappings in proof data without treating assertion strings as artifact claims.

The CLI validates the nonce, original definitions, source IDs, states, and derived counts. A failed assertion counts as finished, so `complete: true` can accompany a failed scenario. Missing, inconsistent, or unfinished proof produces `assertion_proof_missing`, `assertion_proof_invalid`, or `assertion_proof_incomplete`; existing runtime failures remain first. CLI-owned `layout_audit`, `screenshot_match`, and `saved_snapshot_pixels` assertions contribute to `cli_assertions_excluded` and are not claimed as runtime execution proof. An empty runtime set can be complete. `data.completion.assertions_completed` means complete verified coverage; `assertions_passed` is null until proof is complete, then reflects whether any runtime assertions failed. Process exit cleanliness remains separate.

Assertion progress events retain their flattened `index` and add `run_id`, `nonce`, and original `source_id`. If final coverage is unavailable, the CLI may reconstruct bounded diagnostic evidence from matching progress records as `source: "partial_progress"`, `verified: false`, `complete: false`. Stale, partial, or truncated progress cannot substitute for final proof or make a crashed run pass. These rules apply to cold, warm, profiled, and Android scenario execution; historical saved results remain inspectable without implying verified execution coverage.

`scenario sweep` writes ONE aggregate identity with per-seed children under `.qa/artifacts/<scenario>/sweep-<run-id>/`. Each seed's child run is an untouched `scenario run` artifact tree rooted at `seed-<s>/` (repeated seeds share the seed directory with distinct run ids), and the aggregate `sweep.json` at the sweep root records the scenario, the expanded seed sequence, `per_seed[]` entries (`seed`, `ok`, `failed_asserts`, `failure_kinds`, `duration_ms`, `artifact_dir`, `state_records`), and a `summary` (`total`, `passed`, `failed`, `pass_rate`). `state_records` carries the child run's `record_state` evidence map (null when the seed recorded none), so win-rate/balance metrics across a seed range are a single read of `sweep.json` — aggregation stays agent-side, out of the tool.

Bundle artifacts preserve the existing run-level child artifact paths. A bundle summary may add `attempts` arrays to each child entry, but every attempt keeps its own `run_root` instead of overwriting a previous run root. The bundle-level `summary.json` and command JSON `data.runs[]` are where bundle summary attempts, `source_bundle`, `bundle_chain`, launch metadata, cleanup metadata, and retry guidance are written; child `result.json`, `logs.txt`, snapshots, and direct scenario-run artifact maps remain owned by the child scenario run. Bundle summaries also record `godot_args` and redacted `project_args` for each child run and attempt.

Bundle `meta.json` and `summary.json` split by role rather than duplicating one another. `meta.json` is bundle identity and invocation only: `bundle`, `bundle_path`, `command`, `parallelism`, `selection`, `required_bundles`, `required_scenarios`, `optional_scenarios`, and `recommended_manual_checks`. `summary.json` owns the per-run evidence: `runs`, `warnings`, `failures`, and `artifact_index`, alongside the same identity fields plus `selection` and `scheduling`. `artifact-index.json` holds the standalone concise index. `meta.json` no longer repeats `runs` or `artifact_index`; consumers that need child run details read `summary.json` or `artifact-index.json`.

Bundle `summary.json` also carries `timing` and `failure_clusters`, the same two objects the command JSON publishes as `data.timing` and `data.failure_clusters`. `timing` is the run's wall-clock account (`started_at_ms`, `finished_at_ms`, `wall_ms`, `prepare_ms`, `execute_ms`, `artifacts_ms`, `sum_child_duration_ms`, `parallel_efficiency`, `slowest[]`, `pool_seed_ms`, `pool_shutdown_ms`), and each `runs[]` entry carries its own `duration_ms`, `timing` (the child's engine phase breakdown, verbatim), `started_at_ms`, `finished_at_ms`, `lane`, and `queue_wait_ms`, with `duration_ms`, `timing`, `started_at_ms`, and `finished_at_ms` repeated per attempt. The copy in `summary.json` is necessarily a snapshot taken before that file was written — `artifacts_ms` is `0` and `wall_ms` stops short of the summary write — while the command JSON carries the final numbers. `failure_clusters[]` indexes repeated child failures by (`type`, `provider` else `target`) once a group spans at least three scenarios; `failures[]` itself is unchanged except that every child failure now names the `scenario` and `run_root` it came from. Both are additive evidence: they never change `ok`, `failures`, or the exit code.

Bundle preparation is invocation-scoped: manifest discovery, the scenario index,
and normalized selected documents are reused by warm eligibility and dispatch,
including across named bundles. It is a snapshot of those inputs; a subsequent
invocation reads them afresh. `timing.dispatch_wait_ms` records the interval after
preparation and before dispatch, such as pool startup or execution of an earlier
bundle. `started_at_ms` is the actual preparation start, and `wall_ms` includes
that wait; `prepare_ms` contains preparation and scheduling work. Bundle wall
times can therefore overlap in a multi-bundle invocation and must not be summed
as exclusive execution durations.

Bundle command JSON, `meta.json`, and `summary.json` also record `parallelism` for the child process scheduler: `raw_jobs`, `resolved_jobs`, `source`, `available_parallelism`, `reserved_count`, `runnable_count`, and `complete_queued`. This metadata is bundle-level scheduling evidence only; child artifacts remain direct `scenario run` artifacts with isolated run roots.

Bundle command JSON and `meta.json`/`summary.json` also record which children the run was narrowed to, as `selection`: `mode` (`"all"` or `"filtered"`), `only[]`, `failed_only`, `source_run_root` (the previous run directory a `--failed-only` baseline was read from, else `null`), `member_count`, and `selected_ids[]`. The key is always present, so "unfiltered" is stated rather than inferred from a missing key, and `runs[].index` stays the bundle expansion index whether or not the run was filtered. `summary.json` and the command JSON additionally record the dispatch order as `scheduling`: `policy` (`manifest` or `longest_first`), `origin` (`flag`, `config`, or `default`), `applied`, `history_runs`, `predicted`, and `unknown`. Longest-first dispatch reads its per-scenario durations from the newest five readable bundle `summary.json` files, which makes those files an input to the next run and not only a record of the last one; a `summary.json` that cannot be read is skipped rather than failing the run. Both objects are additive scheduling evidence: neither changes `ok`, `failures`, or the exit code.

Bundle `warnings[]` — in the command JSON and in `summary.json` — may also carry `warm_session_start_failed` entries from a `--warm` run: one per pooled warm session that failed to start, each with the original failure kind in `actual`, the session artifact directory in `path` when one existed, and the whole startup failure under `cause`. They are infrastructure evidence, never promoted to failures by bundle `--strict`. The same run's `data.warm_pool` holds the aggregate (`sessions_started`, `session_start_failures`, `transport_failures`, `seed_boot_ms`, `shutdown_ms`, `acquire_wait_ms_budget`).

Bundle runs also write a concise machine-readable artifact index at `artifact-index.json`. The top-level bundle command exposes the same concise summary object as `data.artifact_summary` and records the file path as `data.artifacts.artifact_index_path` alongside `run_root`, `meta_path`, `summary_path`, and `result_path`. The index is additive review evidence: it points to child artifacts without replacing them, and it must not duplicate large screenshot payloads.

Each child entry in `artifact-index.json` and `data.artifact_summary.children[]` uses normalized public field names:

- `scenario`: child scenario name.
- `status`: child outcome such as `passed`, `failed`, `warning`, or `skipped`.
- `run_root`: child scenario artifact root when one was prepared.
- `result_path`: child `result.json` path.
- `logs_path`: child `logs.txt` path.
- `screenshot_paths`: array of screenshot artifact paths.
- `diagnostic_paths`: array of diagnostic artifact paths.
- `failure_artifact_paths`: important failure evidence paths such as screenshot diffs, metrics, masks, overlays, temporal evidence, missing-result targets, or malformed artifact paths when available.
- `source_bundle`: bundle that contributed this scenario.
- `bundle_chain`: ordered bundle lineage for nested bundle expansion.

Artifact retention is opt-in through `artifacts.keep_runs` in `qa/config.yaml` and is off by default: with no setting, run directories accumulate exactly as before and nothing is ever removed. When set to a positive integer `N`, `scenario run` keeps the newest `N` run directories under `.qa/artifacts/<scenario>/` for the scenario it ran, and `scenario bundle run` keeps the newest `N` under `.qa/artifacts/<scenario>/` for every child scenario and the newest `N` under `.qa/artifacts/bundles/<bundle>/`, then removes pooled live-session directories under `.qa/artifacts/live/` that started more than one hour before the oldest retained bundle run.

Retention is deliberately narrow about what it may delete. Run directories are selected by their id — `<epoch>Z` or `<epoch>Z-NN` for scenario runs, a bare `<epoch>` for bundle runs — ordered numerically rather than by mtime, so second-resolution ids minted by parallel bundle children still order correctly and the run just written is always inside the retained window. Anything that does not match those id shapes is never a candidate: files, `sweep-*` aggregates, baselines, and any other directory are left alone. Under `.qa/artifacts/live/`, only `session-<millis>-<hex>` directories are candidates, and only when they are past the age cutoff *and* have no `session.json` or name a process that is no longer running; on Unix, the `live/latest` symlink is never removed and the session it points at is always kept (Windows preserves the equivalent junction) so documented `.qa/artifacts/live/latest/...` paths keep resolving, non-`session-` entries are never touched, and the singleton `.qa/session.json` is outside the pruned tree entirely. That age cutoff carries a one-hour margin, and the margin is not cosmetic: a bundle run id is the epoch second the run *finished* while its pooled sessions start when the run begins and are shut down before the artifacts are written, so pruning at the bare bundle-run id would delete the retained run's own session logs. Each bundle run also rotates `.qa/artifacts/bundles/<bundle>/latest`, a relative symlink on Unix or an NTFS directory junction on Windows to the run directory just written and reported as `data.artifacts.latest_path`. It is a convenience alias in exactly the sense `live/latest` is: the stable evidence path is the run-id directory reported in the command JSON. Retention never removes it and never leaves it dangling — run directories are selected by their epoch-shaped name and by `DirEntry` metadata that does not follow symlinks, so the link is not a candidate, and it always names the newest run, which is always inside the retained window. If a convenience alias cannot be created, use the explicit run directory; bundle alias rotation is best-effort and does not change the validation verdict. Pruning runs after the command's artifacts are written, reports what it removed under `data.retention`, and reports deletion errors as an `artifact_prune_failed` warning without changing `ok`, `failures`, or the exit code. That warning reaches the command JSON `data.warnings[]` only: pruning has to run after the bundle's own run directory exists, which is after `summary.json` has been written, so `summary.json` never carries it.

Because a run only ever prunes what it touched, history accumulates under scenarios and bundles nothing has revisited since `artifacts.keep_runs` was set. `godot-qa artifact prune [--keep-runs <n>] [--dry-run]` applies the same rules to the whole artifacts root on demand — every scenario directory, every bundle, and the pooled sessions under `live/` — and `--dry-run` reports the plan without deleting anything. It is the only command that prunes directories no run of its own produced, so unlike the housekeeping prune above it does report deletion failures as failures: see `docs/cli-spec.md` for its contract.

Missing optional scenarios are represented as warnings in the index instead of being hidden. Required child failures keep their original detailed artifacts: child `result.json`, logs, screenshots, diagnostics, snapshot artifact maps, and failure artifacts remain separate files under the child run root. Nested bundle summaries retain `source_bundle` and `bundle_chain` for every scenario entry so composed gates can be reviewed without losing lineage.

Each bundle attempt object records JSON-compatible metadata:

- `attempt`: one-based attempt number.
- `status`: `passed`, `failed`, or `skipped`.
- `failure_type`: the existing or additive failure type for that attempt.
- `category`: `infrastructure`, `display`, `process`, `artifact`, `gameplay_assertion`, or `log_health`.
- `run_root`: the scenario attempt artifact root.
- `launch_display` or `display_lifecycle`: selected display mode, driver, host display facts, virtual-display wrapper/allocation status, startup status, and shutdown status.
- `cleanup_status`: process cleanup result for processes owned by the current `godot-qa` run, including stale-process detection and whether cleanup completed, failed, or was not needed.
- `retry`: retry eligibility, backoff, bounded attempt count, and whether the retry policy was exhausted.
- `retry_guidance`: actionable command and policy text for infrastructure-only startup failures.

Infrastructure startup failures use distinct failure kinds so missing artifacts are not collapsed into one generic result: `display_startup_exhausted`, `virtual_display_allocation_failed`, `stale_process_cleanup_failed`, and `scenario_result_missing` with `category: "infrastructure"` when the missing result follows display startup failure. Existing `scenario_result_missing`, `godot_run_failed`, `display_backend_unavailable`, `scenario_result_invalid`, runtime assertion failure types, screenshot quality failures, and log-health failures remain compatible; the additive category and attempt metadata clarify the bundle outcome without weakening strict pass/fail behavior.

Generic review summaries produced by `godot-qa artifact summarize <run-root> --json` write `artifact_summary.json` in the inspected run root by default. The same format applies to scenario run roots, live-session roots, bundle roots, and manual review folders. The summary is an index: it references existing artifacts and records small JSON/log excerpts, but it must not copy screenshots, audio, traces, contact sheets, overlays, diffs, or other binary payloads into the summary.

Custom children inside external display wrappers use an opt-in receipt root created by `artifact begin-run`: `run.json` is written before launch, `result.json` is written by the child, `child-status.json` is written by the inner wait adapter after the child completes, and `verdict.json` is written by `artifact validate-result` after the outer wrapper exits. The three input receipts carry `schema_version: 1`, matching `run_id`, random `nonce`, and `started_at_unix_ms`; the two completion receipts also carry `completed_at_unix_ms` at or after run start. An existing root or verdict is never overwritten. `result.json` has boolean `ok` and `failures[]`; `child-status.json` has a conservative `termination` and exit details. A wrapper exit of zero is insufficient without valid matching child receipts. See [the CLI contract](cli-spec.md#artifact-begin-run--validate-result) and the [example inner adapter](../examples/verdict_child_status.sh).

Native one-shot scenario requests also carry an additive random `run.nonce`, and the addon returns `run_identity` in its raw result. Finalized `CommandResult` JSON carries the requested identity under `data.run_identity` and records whether the result echoed it under `data.result_identity_verified`; older saved results remain readable with verification false. The one-shot process runner starts a full three-second exit grace period only after it reads a valid result for the current request identity, even if the original run deadline is near; the original deadline remains hard until that result exists. If the owned process remains alive, it terminates that process group and reports completed assertions separately from clean process completion, preserving the first assertion failure. A timed-out result with stale or absent identity cannot claim completed assertions. The addon writes a nonce-bound `runtime_started` event to `progress.jsonl` before scene loading; any progress in this run blocks bundle startup retries, including when a later log line resembles a display startup error. Raw stdout and stderr are captured to separate run files before being combined into `logs.txt`, so inherited pipe handles cannot stall finalization or erase partial output. In-memory review retains at most the first and last 4 MiB of each stream; a larger stream records `data.process.output_truncation`, fails as `output_capture_truncated`, and keeps the complete raw `.stdout.txt` and `.stderr.txt` files for inspection.

`artifact_summary.json` includes these required category arrays, even when a category is empty:

- `scenario_results`
- `snapshots`
- `screenshots`
- `screenshot_sequences`
- `contact_sheets`
- `screenshot_diffs`
- `layout_audits`
- `traces`
- `physics_inspections`
- `physics_overlays`
- `collision_reports`
- `surface_audits`
- `audio_artifacts`
- `audio_analyses`
- `event_traces`
- `visual_audits`
- `profiles`
- `logs`
- `errors`
- `warnings`
- `unknown_artifacts`

Each artifact entry uses JSON-compatible fields:

- `relative_path`: path from the summarized root.
- `absolute_path`: absolute path only when it is safe and remains inside the summarized root.
- `type`: category name from the public category arrays.
- `size_bytes`: file size.
- `mtime`: filesystem modification time when available.
- `linked_manifest_or_result`: nearest linked manifest or result path when known.
- `important_fields`: compact summary of important JSON fields such as status, rule IDs, metrics, paths, and artifact references.
- `warnings`: warning findings associated with this artifact.
- `failures`: failure findings associated with this artifact.

Required generic summary warning and failure rule IDs:

- `artifact_summary.run_root_missing`
- `artifact_summary.unrecognized_artifact`
- `artifact_summary.result_failure_detected`
- `artifact_summary.runtime_errors_detected`
- `artifact_summary.audio_warning_detected`
- `artifact_summary.visual_warning_detected`
- `artifact_summary.physics_warning_detected`
- `artifact_summary.event_warning_detected`
- `artifact_summary.missing_expected_artifact`
- `artifact_summary.large_artifact_skipped`

When `--markdown-out <path>` is supplied, the Markdown report is compact and review-oriented. It uses these section headings:

- `# Artifact Summary`
- `## Run root`
- `## High-priority findings`
- `## Screenshots and visual artifacts`
- `## Runtime traces and physics`
- `## Audio`
- `## Events`
- `## Logs and errors`
- `## Unknown artifacts`

S9 screenshot mismatches write evidence under `diffs/` using assertion-order labels such as `screenshot-01-after_start.diff.png` plus `screenshot-01-after_start.metrics.json`. Scenario-run screenshot failures attach `actual`, `baseline`, `diff`, and `metrics` paths directly in `failure.artifacts`.

## Recorder artifacts (S39)

`scenario record` writes recorder evidence under:

```text
.qa/artifacts/recordings/<name>/<run-id>/
  recording.json
  candidate.json
  snapshots/
    initial/
    final/
    moment-<frame>/
```

Recorder artifact contract:

- `recording.json` stores the exported bounded/manual input trace and recorder metadata.
- `candidate.json` mirrors the generated candidate scenario, including review-only `review.suggested_assertions`.
- `snapshots/` include deterministic moment snapshots keyed by recorded input moments when non-interactive replay is used.
- Suggested assertion evidence is structured and should include `kind`, `assertion`, `input_moment`, `target`, `snapshots`, and `evidence` payload fields so reviewers can validate why a suggestion exists.
- Screenshot/baseline-related suggestions remain review artifacts only; they must not write to `qa/baselines` and must never imply baseline approval.

`godot-qa screenshot compare` writes metrics in JSON output for every run. On mismatch it writes a diff image and metrics file either at the explicit `--diff` location or under `.qa/artifacts/screenshot-compare/<run-id>/`. `godot-qa screenshot compare --live` requires a windowed live session and reuses the same compare engine after first staging and quality-checking a PNG under `.qa/artifacts/live/<session-id>/screenshots/`.

`result.json.ok` must be a JSON boolean, `duration_ms` must be a non-negative integer when present, `snapshots` must be a JSON array of strings, `artifacts` must be an object with string path values when present, `snapshot_artifacts` must be an object whose snapshot entries are also objects of string path values when present, and `failures` must be a JSON array of objects. Within each failure object, `type`, `message`, `path`, `target`, and `severity` must be strings when present, and nested `artifacts` must also be an object of string paths. The CLI rejects malformed payloads as `scenario_result_invalid` instead of silently coercing or forwarding unstable types.

For aggregated S7 layout runs, `result.json.data.viewports` records the requested viewport list and `result.json.data.runs` records the per-viewport results that produced the flattened top-level `failures` array.

## `request.json`

`request.json.run.command` mirrors the same shell-ready CLI command string so the handoff contract carries the exact invocation context used to prepare the run.

When `scenario run` contains CLI-side assertions such as `screenshot_match`, `request.json` preserves the full declared scenario contract while Godot itself runs against a separate internal stripped request payload.

```json
{
  "scenario": {
    "name": "main_menu_start_opens_pause",
    "scene": "res://ui/main_menu.tscn",
    "steps": [{"wait_frames": 5}]
  },
  "artifacts": {
    "run_root": "/path/to/project/.qa/artifacts/main_menu_start_opens_pause/20260421T120000Z",
    "request_path": "/path/to/project/.qa/artifacts/main_menu_start_opens_pause/20260421T120000Z/request.json",
    "result_path": "/path/to/project/.qa/artifacts/main_menu_start_opens_pause/20260421T120000Z/result.json",
    "logs_path": "/path/to/project/.qa/artifacts/main_menu_start_opens_pause/20260421T120000Z/logs.txt",
    "screenshot_paths": [],
    "diagnostic_paths": ["/path/to/project/.qa/artifacts/main_menu_start_opens_pause/20260421T120000Z/snapshots"]
  },
  "run": {
    "id": "20260421T120000Z",
    "started_at": "2026-04-21T12:00:00Z",
    "command": "godot-qa scenario run main_menu_start_opens_pause --json"
  }
}
```

Live-session `request.json.artifacts` now also includes `screenshot_paths` so the runtime bridge can stage standalone screenshot PNGs without leaving the controlled session artifact root.

## `tree.json`

```json
{
  "root": {
    "path": "/root/MainMenu",
    "name": "MainMenu",
    "type": "Control",
    "qa_id": "main_menu",
    "visible": true,
    "children": []
  }
}
```

When a tree capture is filtered, `tree.json` includes a top-level `filter` object:

```json
{
  "filter": {
    "target": "qa:start_button",
    "resolved_target": "qa:start_button",
    "max_depth": 1,
    "qa_only": true,
    "include_metadata": true,
    "metadata_keys": ["qa_id"]
  },
  "root": {}
}
```

Full snapshots omit `filter` and remain the compatibility/debug path. `artifact query` is most reliable against full tree artifacts; against filtered trees it can only search retained nodes, so missing matches may be omitted evidence rather than absent scene state. Snapshot-backed assertions that can detect this report filtered-evidence failures such as `snapshot_target_omitted`.

## `ui.json`

```json
{
  "viewport": [1920, 1080],
  "controls": [
    {
      "qa_id": "pause_menu",
      "path": "/root/MainMenu/PauseMenu",
      "type": "PanelContainer",
      "visible": false,
      "global_rect": [720, 360, 480, 240]
    },
    {
      "qa_id": "start_button",
      "path": "/root/MainMenu/Center/MenuPanel/StartButton",
      "type": "Button",
      "text": "Start",
      "visible": true,
      "disabled": false,
      "has_focus": true,
      "focus_mode": "all",
      "global_rect": [760, 520, 400, 72],
      "minimum_size": [120, 40],
      "anchors": [0.5, 0.5, 0.5, 0.5],
      "mouse_filter": "stop",
      "theme_type_variation": "PrimaryButton"
    }
  ]
}
```

`ui.json` now includes every `Control` under the captured scene subtree, not only visible controls. Hidden controls stay in the payload with `visible: false` so snapshot-backed assertions can distinguish `hidden` from `not_exists` and still inspect disabled/text-bearing controls that are currently not shown.
Filtered UI captures include a top-level `filter` object with the same fields as `tree.json` and only include controls retained by the scoped/depth/QA filter.

## `focus.json`

```json
{
  "focus_owner": "qa:start_button",
  "focus_owner_path": "/root/MainMenu/Center/MenuPanel/StartButton",
  "graph": [
    {
      "target": "qa:start_button",
      "neighbors": {
        "down": "qa:settings_button"
      }
    }
  ]
}
```

Filtered focus captures include the same top-level `filter` object when filters are active. Focus owner and graph data are limited to retained controls; use a full snapshot when focus traversal outside the scoped subtree matters.

## Failure objects

Every failure should include:

```text
type
message or structured expected/actual
target when applicable
severity
artifact paths when applicable
```

## Temporal evidence artifacts (S33)

Temporal assertions write bounded evidence JSON files under `temporal/` in the run root.

Example shape:

```json
{
  "assertion": "max_delta_position",
  "target": "qa:demo_actor",
  "window": {"frames": 12},
  "sampling": {"sample_every_frames": 1},
  "expected": {"max_delta_position": 32},
  "actual": {"max_delta_position": 57.2},
  "failed_sample": {"index": 9, "frame": 118, "time_seconds": 1.966},
  "samples": [
    {"index": 7, "frame": 116, "time_seconds": 1.933, "position": [288, 208]},
    {"index": 8, "frame": 117, "time_seconds": 1.95, "position": [314, 208]},
    {"index": 9, "frame": 118, "time_seconds": 1.966, "position": [345, 208]},
    {"index": 10, "frame": 119, "time_seconds": 1.983, "position": [346, 208]}
  ],
  "context": {"before": 2, "after": 1}
}
```

Bounded evidence requirements:

- Evidence is bounded in size by window and sampling configuration.
- Failures should include nearby samples before/after the failing sample for diagnosis.
- Failure payloads reference evidence via `failure.artifacts.temporal_evidence`.

Example:

```json
{
  "type": "focused_mismatch",
  "target": "qa:resume_button",
  "expected": "qa:resume_button",
  "actual": "qa:settings_button",
  "severity": "error",
  "artifacts": {
    "ui": ".qa/artifacts/pause/latest/snapshots/after_start/ui.json",
    "screenshot": ".qa/artifacts/pause/latest/snapshots/after_start/screenshot.png"
  }
}
```

When a runtime failure already points at snapshot-local files, the CLI preserves those keys and merges in the prepared run-level paths (`run_root`, `result_path`, `logs_path`, `screenshot_paths`, and `diagnostic_paths`) so one failure payload still gives both the specific evidence and the enclosing run context.

## Telemetry artifacts

`telemetry record` writes its recording under the live session artifact root at `telemetry/<telemetry-id>/`, and `telemetry/latest` links the newest recording:

- `telemetry.jsonl`: one JSON object per line in sample order. Every row has `frame` (engine process frame), `t_ms` (engine ticks in milliseconds), and `value` (the sampled provider/method value; JSON-compatible). Failed samples keep the row with `value: null` plus an `error` string (for example `missing_method` or `provider_runtime_error`).
- `manifest.json`: small recording summary with `telemetry_id`, `session_id`, `scene`, the sampled source (`provider`, or `target` + `method`), `frames`, `every`, `rows` (row count), `truncated`, `provider_error_count`, and the `artifacts.rows` path.

`--out <path>` copies `telemetry.jsonl` to the requested path in addition to the canonical session artifact; the command result records it as `artifacts.out`.

## Android live artifacts

Android runtime artifacts are created privately under
`user://godot_qa/live/<session-id>` and materialized into the corresponding host
live artifact directory beneath `.qa/artifacts`. The CLI translates outgoing
artifact names to the controlled device root and recursively rewrites returned
paths in success data, failure details, and nested artifact objects.

Materialization covers PNG screenshots, snapshot JSON, WAV audio, profiles,
traces, event and telemetry output, visual and temporal evidence, recorder
output, and patch artifacts. Pulls use binary-safe
`adb exec-out run-as <package> cat`; host writes are atomic. A returned device
path must be strictly beneath the exact session root. Traversal, the root itself,
unrelated `user://` locations, relative path segments containing `:`, and partial transfers fail with actionable
command, expected root, actual path, and destination details. Host
`request.json`, deployment logs, logcat, session metadata, and materialized
artifacts never contain the bridge token.

On Windows hosts, Android session metadata may include `logcat_process_identity`, an opaque unsigned 64-bit process-creation timestamp alongside `logcat_pid`. Unix records omit it and older records remain readable. Cleanup requires matching identity before terminating the host logcat process; missing or mismatched ownership produces `android_logcat_stop_failed` rather than terminating an unrelated reused PID.

Android one-shot `scenario run` places its run beneath the session root at
`user://godot_qa/live/<session-id>/run`. The CLI pushes the rewritten
`run/request.json` into private storage, pre-creates the `run/snapshots`,
`run/temporal`, and `run/audio` directories, then pulls `run/result.json` and
best-effort `run/progress.jsonl` back into the normal host `.qa/artifacts` run
tree. The device result echoes the request's artifact map (device paths); the CLI
substitutes the whole run-level artifact map for the host layout — so host-only
files such as `logs.txt` and `meta.json` are never pulled — and materializes only
the real claimed evidence (snapshot bundles, screenshots, WAVs, temporal, and
`failures[].artifacts`).

## Presented startup captures

`screenshot assert-startup-presented` creates a new `.qa/artifacts/presented-startup/<run-id>/` (or explicit `--out`) directory. `presented.png` contains the owned compositor capture; `viewport.png` contains subsequent real Godot viewport pixels, including blank pixels when available. `diff.png` and `metrics.json` come from the existing comparator. Dimension mismatches are failures without resizing.

Presented comparison metrics include the opt-in `channel_tolerance` (default 0), raw and tolerated different-pixel counts/ratios, and maximum channel delta. `different_pixels` and `different_ratio` consistently report the thresholded decision; the raw diff PNG remains unmodified.

`helper-result.json` records the isolated child's result. `result.json` is the parent's finalized verdict, including wrapper exit and teardown checks, and is the result to select for evidence export. Capture metadata records source/order, monotonic times, actual display backend, process/session/compositor identities, dimensions, forced-draw status and original PNG hashes; an explicit baseline also has a recorded hash. A missing capture remains missing, with a failure, rather than being replaced with diagnostic pixels. `owner.json` is an internal nonce marker; it and raw wrapper/session files are not curated evidence.

## Android APK cache

`session start --target android` caches debug-APK exports under
`.qa/cache/android/<key>/`, where `<key>` is a SHA-256 over the Godot binary and
version, the export preset and package, and a size+mtime manifest of the project
tree. Each entry directory holds:

- `session-debug.apk` — the cached debug APK, copied into the session deployment
  directory on a hit.
- `meta.json` — `version`, `key`, `created_at_ms`, `last_used_at_ms`,
  `godot_binary`, `godot_version`, `preset`, `package`, `manifest_files`,
  `apk_bytes`, and `tool_version`.

The newest four keys are retained; older entries are evicted on the next store.
This tree is derived data: it is safe to delete at any time (the next start simply
re-exports), and it never contains the session token.

## Verified asset trial records and evidence

An asset trial keeps its durable journal, immutable candidate copies, original
scene backup, and publication scratch files under the selected project's
`.qa/asset-trials/<trial-id>/`. The backup remains the recovery source while the
trial is active. Journals, backups, and evidence remain available after a trial
is kept or discarded; automatic artifact retention never removes them.
Journal integrity is checked before recovery. Missing backups or corrupt records
refuse automatic restoration; do not edit journals to bypass a conflict.
Scene and media publication record intent before changing paths, and captured
bytes remain in hidden storage when a competing edit prevents safe recovery.

Before and candidate evidence is written under the configured artifact root at
`asset-trials/<trial-id>/before/` and `after/`. These roots are pinned against
ordinary retention pruning. Trial evidence captures the ordinary scenario
result plus the selected snapshot or WAV, read-only state checkpoints, and
runtime binding metadata. PNG metadata records the actual runtime node, slot,
resource identity and imported pixel fingerprint. Audio metadata records the
actual node and stream, bus, playback overlap, audio driver and imported stream
fingerprint. Sprite visibility is sampled over the runtime sprite's transformed
node bounds, so transparent pixels in the center of a texture do not hide
visible pixels elsewhere. Audio metadata also records whether the stream was
paused; playback overlap requires that the player was active and unpaused on the
captured bus. Missing pixels, an unavailable real audio driver, or playback that
does not overlap the capture is a verification failure rather than inferred
success. Evidence directories must be outside Godot's scanned resource tree or
already protected by a project-owned `.gdignore`.
