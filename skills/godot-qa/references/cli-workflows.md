# CLI workflows

Use this reference for setup, one-shot checks, and live investigation. Every command below selects a Godot project explicitly; replace example scenes, locators, and filenames with project values.

## Setup and focused checks

`doctor --project <project> --json` checks readiness and reports non-failing warnings, including incompatible binaries on `PATH`. Keep the CLI and copied/linked addon versions aligned. In source-based development, identical version strings do not prove identical builds: confirm which executable runs and which checkout supplied the copied or linked addon. For initial setup, copy the matching `addons/godot_qa` into the project's `addons/` directory and enable the Godot QA plugin. Keep QA mode test-only; normal game runs and release exports must not expose the bridge.

Configuration lives under the selected project: `qa/config.yaml` and optional ignored `qa/config.local.yaml`. Local overrides take precedence; inspect them before changing launch options. On a fresh checkout, run:

```bash
godot-qa import --project <project> --json
godot-qa check --project <project> --scene res://ui/main_menu.tscn --json
godot-qa check --project <project> --script res://src/player.gd --json
godot-qa check --project <project> --tests --json
```

Choose only the checks relevant to the task. `check --tests` runs explicit `qa/tests/manifest.json` entries; it does not discover arbitrary test frameworks. `check --all-scenes` may encounter intentional negative fixtures. Script load/compile checks do not instantiate the script or exercise gameplay.

`install-skills --project <project> --json` installs the package embedded in that executable into `.agents/skills/godot-qa`. After updating skill sources, rebuild the CLI before reinstalling; an older binary embeds older instructions. Reinstallation replaces that skill directory, so maintain custom instructions elsewhere.

## Live inspection and input

Check `session status` before starting or reusing a session. Inspect `tree`, `ui`, `focus`, or `focus graph` to locate controls; `node inspect <locator>` gives focused runtime state. These commands accept `--project <project> --json`.

For gameplay inputs, choose `input tap`, `input hold`/`input release`, `input click`, `input key`, `input type`, or `input joy button`/`input joy motion`. Mouse wheel, touch, and drag commands are also available; inspect their `--help` for coordinates, durations, and device IDs. Release held input after a test.

Successful input dispatch does not establish that the game handled it; assert the resulting UI or semantic state. For resolution-dependent clicks, reproduce the same initial state and locator interaction at each relevant `session start --viewport WIDTHxHEIGHT`, then inspect rendered framing, transforms, focus, and overlays. Locator clicks account for viewport/canvas scaling and support Controls in the active root window viewport; a target in a SubViewport is unsupported. Do not treat logical UI rectangles as raw mouse coordinates or replace a failing mouse test with keyboard navigation when mouse behavior is the contract.

Windowed realtime sessions are the default. For non-visual automation only:

```bash
godot-qa session start --project <project> --headless --fastest --fixed-fps 60 --json
```

`--fixed-fps` requires `--fastest`. Game arguments use repeated `--project-arg`, for example `--project-arg --seed --project-arg 123456`. Prefer existing project configuration over inventing launch flags.

## Sequenced reviews and reload

For a saved live script, use:

```bash
godot-qa session run-script .qa/review-scripts/current_live.yaml --project <project> --json
```

Live scripts contain a `steps` array of single-operation objects. This read-only example inspects an existing session:

```yaml
steps:
  - tree: {}
  - ui: {}
  - errors: {}
```

Use `review list-templates` and `review init --help` to generate more involved scenario harnesses or live scripts; generated files are not auto-run. Review their steps before execution, especially `session_start` and `session_stop`: requesting windowed mode can replace an existing headless session. Saved scenario harnesses run with `scenario run --file <path>`.

A `session_start` step supports `project_args`, and `frame_pacing: {mode: fastest, fixed_fps: 60}` or `fastest: true`. For fixture setup, use `fixture_call` or `call_method` for explicit project-owned `qa_` methods. Use read-only assertions for observations, not `edit-live call`. The runner executes sequentially and stops at the first failed step.

`session reload` reloads the active scene and clears held input, live edits, and runtime errors. `session watch` reloads on project file changes. Inspect evidence before reload if it is needed to diagnose a failure. Use `session start --hot-reload` when Godot debugger-backed script/resource reload is needed.

## Temporary edits and failures

For an authorized experiment, use `edit-live set` or the allowlisted `edit-live call`, then inspect `edit-live diff`. Persist intentionally through `edit-live export-patch` and `apply-patch`; inspect command help and the patch before application. Prefer Godot-applied structured patches over raw `.tscn` editing.

Keep live commands serialized per session. For `session_bridge_busy`, let the current request finish before retrying. For unreachable sessions, connection timeouts, or invalid tokens, inspect session status and the running process; do not loop or print token contents. Malformed runtime results and artifact-write failures require runtime/filesystem diagnosis. Preserve failure evidence before stopping a session you own.

Optional deeper contracts: `docs/cli-spec.md` and `docs/patch-format.md` in a matching godot-qa checkout or distribution.
