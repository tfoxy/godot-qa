# Visual and artifact review

Use this reference when the claim depends on rendered appearance, layout, or an evidence packet. Validate actual pixels and structured runtime data; a successful file write alone is not visual acceptance.

## Capture and compare

Check the active session and use windowed rendering for screenshots. `visual preflight --project <project> --json` diagnoses whether the selected launch mode can provide real pixels. Headless placeholders and low-information images are not valid screenshot evidence; resolve the display/capture failure rather than accepting them.

```bash
godot-qa screenshot capture --project <project> --out .qa/artifacts/manual/current.png --json
godot-qa screenshot compare --project <project> --actual .qa/artifacts/manual/current.png --baseline qa/baselines/current.png --json
```

For an existing approved baseline, `screenshot compare --live --baseline <path>` captures and compares the active windowed session. To compare saved scenario evidence, use its returned screenshot path with `--actual`. Pass `--project <project> --json` in either case. Read the result and inspect actual, baseline, and diff images; baseline approval remains an explicit human action.

Capture guards `--require-visible`, `--require-hidden`, and `--require-state` can ensure the intended UI state before saving. Use `--setup-script <path>` only for reviewed setup steps. For transitions, use `screenshot sequence --frames N --every N --out <dir>` or `--seconds N --fps N`, then `screenshot contact-sheet <dir> --columns 5 --thumb-width 320 --out <png>`. Contact sheets consume the sequence manifest and its ordered frames, not an arbitrary image folder.

When per-image bridge calls or PNG encoding interfere with a transition, opt into a rendered burst:

```bash
godot-qa screenshot sequence --capture-mode burst --frames 12 --every 2 --out .qa/artifacts/manual/burst --project <project> --json
```

Serial remains the default. Burst retains RGBA frames from actual post-draw events, then encodes them after sampling. Burst-only `--max-bytes` and `--max-seconds` bound retained RGBA bytes and capture time; defaults are 512 MiB and three seconds, with hard limits of 1 GiB, ten seconds, and 96 retained frames. Choose limits for the viewport size and requested schedule. `--frames` is the frame-window endpoint: the example samples ordinals 0, 2, …, 12 (seven images), not twelve images. Frame schedules follow draw ordinals; time schedules use monotonic deadlines. Inspect observed engine frames, draw ordinals, timing, effective limits, and completeness instead of assuming requested FPS was achieved.

The burst owns the serialized bridge while rendering continues. Arrange inputs or autonomous motion before starting; other CLI input/capture commands cannot run concurrently on that session. Guards and stop predicates run inside the burst. Capture overhead remains, cancellation is not responsive, and a process crash can lose in-memory frames. Resize, budget, or deadline failures produce incomplete evidence; inspect retained partial artifacts and preserve the failure. Use a stable viewport and bounded samples rather than treating the sequence as unperturbed animation timing.

For what the user sees at startup, `screenshot assert-startup-presented --scene res://... --viewport 1280x720` captures compositor pixels before a QA viewport capture. It requires Linux Gamescope and an owned display. Without a baseline, it compares presented pixels against the later viewport image; with `--baseline`, it checks that image. Exact channel comparison is default; `--channel-tolerance 1` allows one 8-bit rounding level independently of the differing-pixel threshold. Do not loosen tolerances simply to pass.

## Layout and perception

```bash
godot-qa layout audit --project <project> --scene res://ui/settings_menu.tscn --json
godot-qa layout audit-current --project <project> --scenario <scenario> --live --json
```

Use fresh scene audits for startup geometry and current audits for state reached through interaction. When comparing resolutions, check actual screenshot dimensions, crop/letterboxing, and whether the intended content is in frame before attributing a difference to game layout. `visual audit` measures contrast/readability for explicit targets. `visual isolate` compares layers through a project visual-layer provider plus fixture driver. Target-hidden pixel assertions and layer isolation compare sequential captures: prepare a stable visual state when animation, camera movement, or particles would otherwise contaminate the differences. Inspect command help and existing project adapters before invoking these specialized workflows.

## Find and interpret evidence

```bash
godot-qa artifact summarize <run-root> --json
godot-qa artifact query <run-root> --snapshot <name> --qa-id start_button --visible --json
```

`summarize` indexes evidence; use `--no-write` when reviewing without creating its index artifact. Narrow with `--type`, `--warnings-only`, or `--expect` as needed. Queries inspect saved snapshots, not the current game.

Default run locations are `.qa/artifacts/<scenario>/<run-id>/` and `.qa/artifacts/live/<session-id>/`. Snapshot bundles normally include `tree.json`, `ui.json`, `focus.json`, `errors.json`, and, when enabled, `screenshot.png`. Follow returned paths for configured artifact roots. Bind findings to the selected project, source revision/build when known, run identity, and capture time; historical evidence may describe an earlier source state. Keep failed runs when rerunning or sharing evidence, and distinguish their findings from current results.

## Portable export

Create an explicit selection file, then run `artifact export <selection.json> --out <new-directory> --json`. Example selection, with real run/media paths substituted; omit image or audio selections when unneeded:

```json
{
  "schema_version": 1,
  "title": "Menu review",
  "sources": [{
    "id": "menu",
    "run_root": ".qa/artifacts/menu_start/run-id",
    "result": "result.json",
    "images": [{"id": "after", "path": "snapshots/after/screenshot.png", "role": "original"}],
    "audio": [{"id": "menu_sound", "path": "audio/menu.wav", "role": "original"}]
  }]
}
```

Relative run roots resolve beside the selection file; selected media paths are confined to each run root. Select finalized `result.json` or external `verdict.json`, exact PNGs, optional WAVs, and optionally scalar profile fields. WAV selections require `role: original`, valid bounded WAV data, and at most 64 MiB per file; packet limits are 64 WAVs and 512 MiB of audio. Export copies original WAV bytes, records source/export SHA-256 plus audio metadata and `byte_exact_copy`, and creates relative playback/download links. This packages saved evidence; it does not invent live-capture provenance or transcode it.

Images and audio default to unreviewed (`visual_review` and `audio_review`). An explicit selection `review` with status `reviewed` must bind to the source `sha256` and supply `reviewer`, `method`, `scale`, `scope`, and `reviewed_at`; copying or analyzing media does not perform that review. Export preserves failed/incomplete source verdicts and produces relative links without raw requests, logs, or connection data. A portable packet does not turn a failed run into acceptance or approve baselines.

## External display wrappers

A wrapper exiting zero does not establish that its child passed. For custom children:

1. Run `artifact begin-run --out <new-run-root> --json` before launch.
2. Pass its identity fields to the child and an adapter inside the wrapper. The child writes `result.json`; the adapter waits for the actual child and atomically writes `child-status.json` with the real termination status. Preserve the returned run ID, nonce, and timestamps in both receipts.
3. After the wrapper exits, run `artifact validate-result <run-root> --wrapper-exit <code> --json`. Use `--wrapper-timed-out` only when the orchestrator actually enforced that deadline. Inspect the resulting verdict.

Implement receipt adapters against the exact schema, not guessed exit-code heuristics. Optional deeper sources in a matching godot-qa checkout/distribution: `docs/cli-spec.md` (external receipts and export), `docs/artifact-format.md`, `examples/verdict_child_status.sh`, and `examples/evidence-selection.json`.
