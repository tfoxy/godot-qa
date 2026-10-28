# Visual and artifact review

Use this reference when the claim depends on rendered appearance, layout, or an evidence packet. Validate actual pixels and structured runtime data; a successful file write alone is not visual acceptance.

## Capture and compare

Check the active session and use windowed rendering for screenshots. `visual preflight --project <project> --json` diagnoses whether the selected launch mode can provide real pixels. Headless placeholders and low-information images are not valid screenshot evidence; resolve the display/capture failure rather than accepting them.

```bash
godot-qa screenshot capture --project <project> --out .qa/artifacts/manual/current.png --json
godot-qa screenshot compare --project <project> --actual .qa/artifacts/manual/current.png --baseline qa/baselines/current.png --json
```

For an existing approved baseline, `screenshot compare --live --baseline <path>` captures and compares the active windowed session. To compare saved scenario evidence, use its returned screenshot path with `--actual`. Pass `--project <project> --json` in either case. Read the result and inspect actual, baseline, and diff images; baseline approval remains an explicit human action.

Capture guards `--require-visible`, `--require-hidden`, and `--require-state` can ensure the intended UI state before saving. Use `--setup-script <path>` only for reviewed setup steps. For transitions, use `screenshot sequence --frames N --every N --out <dir>` or `--seconds N --fps N`, then `screenshot contact-sheet <dir> --columns 5 --thumb-width 320 --out <png>`. Contact sheets consume the sequence manifest and its ordered frames, not an arbitrary image folder. Run captures sequentially on a session. Captures add runtime work; use bounded samples and inspect recorded frame/time evidence before interpreting a sequence as unperturbed animation timing.

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

Create an explicit selection file, then run `artifact export <selection.json> --out <new-directory> --json`. Example selection, with real run/image paths substituted:

```json
{
  "schema_version": 1,
  "title": "Menu review",
  "sources": [{
    "id": "menu",
    "run_root": ".qa/artifacts/menu_start/run-id",
    "result": "result.json",
    "images": [{"id": "after", "path": "snapshots/after/screenshot.png", "role": "original"}]
  }]
}
```

Relative run roots resolve beside the selection file. Select finalized `result.json` or external `verdict.json`, exact PNGs, and optionally scalar profile fields. Image reviews must bind to the source SHA-256. Export preserves failures, marks images unreviewed by default, and produces relative links without raw requests, logs, or connection data. It does not approve baselines.

## External display wrappers

A wrapper exiting zero does not establish that its child passed. For custom children:

1. Run `artifact begin-run --out <new-run-root> --json` before launch.
2. Pass its identity fields to the child and an adapter inside the wrapper. The child writes `result.json`; the adapter waits for the actual child and atomically writes `child-status.json` with the real termination status. Preserve the returned run ID, nonce, and timestamps in both receipts.
3. After the wrapper exits, run `artifact validate-result <run-root> --wrapper-exit <code> --json`. Use `--wrapper-timed-out` only when the orchestrator actually enforced that deadline. Inspect the resulting verdict.

Implement receipt adapters against the exact schema, not guessed exit-code heuristics. Optional deeper sources in a matching godot-qa checkout/distribution: `docs/cli-spec.md` (external receipts and export), `docs/artifact-format.md`, `examples/verdict_child_status.sh`, and `examples/evidence-selection.json`.
