# Workflows and capabilities

`godot-qa` is a runtime QA toolkit for Godot projects. It helps you validate real UI flows and interactive scenes by running Godot, simulating input, inspecting live scene/UI state, capturing screenshots and logs, and returning structured JSON that humans, CI, and AI agents can all use.

## Capabilities

`godot-qa` is a Rust CLI/library (`crates/`) plus a Godot GDScript addon (`addons/godot_qa`). The Rust binary is the supported CLI install/use path.

- **Scenario execution and regression gates** — one-shot `scenario run` with replayed snapshots, assertions, and artifact-backed screenshot diffs (runs only pass when the runtime result, process exit status, and captured engine logs are all clean); `scenario bundle run` composes declarative regression gates from `qa/gates/`, including bounded parallel execution (`--jobs`) and an opt-in `--warm` mode that reuses one Godot process across bundle scenarios for a faster dev loop (scenarios needing process isolation fall back to the cold path automatically; an optional `--equivalence-check` cross-validates warm vs. cold verdicts, with cold remaining authoritative).
- **Live sessions and hot reload** — `session start/status/wait/stop/pause/resume/step` over localhost with authenticated requests; engine-faithful hot reload (`session start --hot-reload`) via a debug-host sidecar that speaks Godot's remote-debugger protocol, soft-reloading live scripts (state preserved), resources, and `.tscn` scene *structure* (nodes added/removed/reparented plus property changes) without restarting the session — `session reload` and `session watch` drive it. Script reload works in any build; resource and structural reload are best-effort/unconfirmed and need a debug or editor build.
- **Runtime inspection** — scene tree/UI/focus (`tree`, `ui`, `focus`, `focus graph`, `node inspect`), structured runtime errors and logs, `physics inspect`/`physics overlay`/`physics collision-report` for collision-feel evidence, provider-driven `surface audit` for walkability/clearance, and `event trace`/`event summarize` for VFX/SFX/UI correlation — all backed by explicit project-owned QA methods, never by inference from node names or colors.
- **Screenshots and visual review** — `screenshot capture/compare/sequence/contact-sheet`, owned Gamescope startup presentation checks with `screenshot assert-startup-presented`, `visual audit`/`visual preflight`/`visual isolate` (per-layer visual isolation with contact sheets), and screenshot-backed occlusion/z-order pixel assertions (`visible_pixels`, `not_occluded`, `renders_above`, `renders_below`). Baseline approval always stays a separate, manual, human action.
- **Audio** — live `audio capture`, project-owned `audio export`, deterministic offline `audio render` from a `qa_` fixture method, and `audio analyze` for measurable facts (silence, clipping risk, transient count, spectral energy). None of it judges subjective sound quality.
- **Assertions** — temporal, proximity, surface-probe/surface-matches, and `qa-state validate` against registered or legacy QA-prefixed provider/state methods with explicit JSON Schemas.
- **Controller lifecycle fixtures** — existing virtual joypad operations provide a callback-consistent roster, reject native device ID collisions, and release injected buttons and axes on disconnect, scene changes and cleanup. Synthetic lifecycle evidence remains separate from physical-controller acceptance.
- **Review workflow and artifacts** — `review list-templates`/`review init` generate ordinary scenario or live-script files that run through existing commands; `artifact summarize` is the review index for scenario, live, bundle, and manual artifact roots; `artifact begin-run`/`validate-result` verify external child receipts; `artifact export` creates portable selections with original images, curated summaries and separate review records. `live-gate` commands check hidden-overlay/inspection gates declaratively.
- **Editor and profiling** — `diagnostics editor`, `profile editor export`, and profiling for sessions, scenarios, and manifest-backed checks. A shared versioned `--soak` specification adds bounded real-time windows, epoch-aware simulation throughput, pause accounting, gauges and retention budgets to scenario and live-session profiles.
- **MCP** — a thin stdio wrapper over the same CLI/library behavior.
- **Bundled AI skill** — `install-skills` installs a project-scoped skill package from `skills/godot-qa/`.

For the exhaustive shipped command inventory, see [`docs/cli-spec.md`](cli-spec.md).

This project is especially useful when source code alone is not enough to tell you whether a menu, layout, or interaction actually works. AI-assisted development is a major use case, but the tool is not limited to AI workflows.

Collision-feel reports provide runtime evidence for human playability review, including bounce, repeated-contact, stuck-like, and soft-contact metrics. They complement review instead of replacing it, and high bounce is not a failure unless the caller opts into thresholds or strict warnings.

## Who This Is For

- Godot developers who want reproducible runtime checks for menus, HUDs, focus navigation, and other UI flows
- Teams building automation around real Godot state instead of brittle file inspection
- Agent or CI workflows that need machine-readable results and artifact paths
- Anyone evaluating whether a Godot QA tool can inspect live scenes, simulate input, and report actionable failures

## Not A Fit If

- You want a polished, production-ready testing platform with broad ecosystem integrations today
- You only need static linting or editor-time validation
- You want screenshot baselines to be auto-approved without human review

## What You Can Do Today

For the exact shipped command inventory, use [`docs/cli-spec.md`](cli-spec.md). The summary below is intentionally high-level rather than a second exhaustive command list.

- Run saved scenarios against a real Godot project with `--json` output
- Profile saved scenarios with runtime counters and per-step timing, without host profiler privileges
- Lint saved scenarios statically for manual-play integrity risks (`scenario lint`) with warning-by-default and strict-mode enforcement
- Run `godot-qa import` before CI or local runtime validation if you want an explicit asset-import step
- Run bounded project checks against one scene or every project scene without turning `check` into a general test runner by default
- Run explicit manifest-backed scene/script checks with `check --tests` when a project opts in via `qa/tests/manifest.json`
- Inspect runtime scene tree, UI geometry/text, focus state, logs, and screenshots
- Simulate stable input flows including saved scenario action/joypad steps, live action tap/hold/release, target clicks, viewport mouse clicks, named keys, and bounded joypad buttons/motion
- Run layout audits across viewport sizes using runtime-derived UI data
- Compare screenshots against approved baselines (`screenshot compare`)
- Capture live-session screenshots and compare current live output against approved baselines
- Capture screenshot sequences and contact-sheet review artifacts while keeping baseline approval manual
- Run optional `visual audit` checks over saved screenshots, live sessions, sequences, or scenario artifacts to collect target size, contrast, viewport, and annotation evidence for manual review; findings are warning-only by default and are not an aesthetic pass/fail oracle
- Validate visual occlusion/z-order contracts with screenshot-backed assertions (`visible_pixels`, `not_occluded`, `renders_above`, `renders_below`) for deterministic 2D/UI targets, while keeping baseline approval manual
- Preflight visual capture support with `visual preflight --json`, and validate useful object pixels from saved snapshot screenshots with `saved_snapshot_pixels`
- Optionally flag screenshot artifact-backed visual-risk warnings during `scenario run --visual-risk`, with strict failure only when `--visual-risk-strict` is requested
- Validate explicit project-owned surface/walkability classifications with bounded QA probe assertions (`assert surface-probe`, `assert surface-matches`) using registered probe providers or legacy QA-prefixed provider methods
- Validate registered state providers, project-owned QA state methods, and saved QA state artifacts with explicit schemas using `qa-state validate`
- Start a longer-lived local session for live snapshotting and inspection
- Use MCP to expose a bounded wrapper over the same CLI behavior
- Install the bundled project-scoped AI skill with `godot-qa install-skills`

## How It Works

`godot-qa` has two parts:

- A Rust CLI/library implementation that resolves scenarios, launches commands, collects artifacts, and reports stable JSON results without changing the public CLI contract.
- A Godot addon that runs inside your project and acts as the source of truth for live runtime state

```text
human / CI / agent
        |
        v
    godot-qa CLI
        |
        v
 Godot project with
 addons/godot_qa enabled
        |
        +-- scene tree
        +-- UI geometry and text
        +-- focus state
        +-- simulated input
        +-- screenshots
        +-- logs and assertion results
```

## Quick Start

The fastest way to evaluate the repo is to use the checked-in sample project in [`examples/sample_project`](../examples/sample_project/README.md).

1. Install the CLI:

   ```bash
   cargo xtask prepare-sample
   cargo install --path crates/godot-qa-cli
   ```

   Or download the published archive for your platform, extract it, and put `godot-qa` on `PATH`.

2. Run a basic environment check:

   ```bash
   godot-qa doctor --project examples/sample_project --json
   ```

3. List the bundled example scenarios:

   ```bash
   godot-qa scenario list --project examples/sample_project --json
   ```

4. Inspect a scoped subtree without collecting the full scene tree:

   ```bash
   godot-qa tree --project examples/sample_project --target qa:start_button --max-depth 1 --json
   ```

5. Run a sample scenario:

   ```bash
   godot-qa scenario run sample_main_menu_start --project examples/sample_project --json
   ```

   To collect runtime profiling evidence for the same scenario:

   ```bash
   godot-qa scenario profile sample_main_menu_start --project examples/sample_project --json
   ```

   Or run a layout-focused sample:

   ```bash
   godot-qa scenario run sample_settings_layout --project examples/sample_project --json
   ```

6. Run a layout audit against the sample settings screen:

   ```bash
   godot-qa layout audit --project examples/sample_project --scene res://ui/settings_menu.tscn --json
   ```

7. Run bounded project validation:

   ```bash
   godot-qa import --project examples/sample_project --json
   godot-qa check --project examples/sample_project --scene res://ui/main_menu.tscn --json
   godot-qa check --project examples/sample_project --all-scenes --json || true
   godot-qa check --project examples/sample_project --tests --json
   ```

If you do not want to install globally from a repo checkout, you can also run the checked-out binary with `cargo run --bin godot-qa -- ...` or `target/release/godot-qa ...` after `cargo build --release --bin godot-qa`.

Screenshot sequence review:

```bash
godot-qa screenshot sequence --project examples/sample_project --scenario screenshot_sequence_demo --frames 120 --every 20 --out .qa/artifacts/manual/sequence_demo --json
godot-qa screenshot contact-sheet .qa/artifacts/manual/sequence_demo --columns 3 --thumb-width 320 --out .qa/artifacts/manual/sequence_demo_contact_sheet.png --json
```

`screenshot sequence --scenario <name>` resolves the scenario for manifest metadata and discoverability, but it does not replay scenario steps or assertions. The sequence manifest and contact-sheet companion JSON point to artifact files; they never embed screenshot pixels or auto-approve baselines.

Visual audit review:

```bash
godot-qa visual audit --screenshot .qa/artifacts/manual/current.png --targets .qa/artifacts/manual/current.targets.json --json
godot-qa visual audit --project examples/sample_project --live --target qa:readable_target --annotate-out .qa/artifacts/manual/visual_audit.png --json
godot-qa visual audit --sequence .qa/artifacts/manual/sequence_demo/manifest.json --target qa:readable_target --json
```

`visual audit` reports measurable target evidence such as `pixel_area`, `contrast_ratio_estimate`, `local_color_distance`, and `inside_viewport`. It is for manual readability review, not aesthetic approval, and it does not update or approve screenshot baselines. Use `--strict-warnings` only when a workflow intentionally wants warning IDs to fail the command.

Review template examples:

```bash
godot-qa review list-templates --json
godot-qa review init scenario-smoke --project examples/sample_project --name smoke_review --scene res://ui/main_menu.tscn --out .qa/review-scenarios/smoke_review.yaml --json
godot-qa scenario run --file .qa/review-scenarios/smoke_review.yaml --project examples/sample_project --json
godot-qa review init deterministic-runtime-review --project examples/sample_project --name deterministic_review --scene res://ui/main_menu.tscn --project-arg --seed --project-arg 123456 --frames 60 --out .qa/review-scripts/deterministic_review.yaml --json
godot-qa session run-script .qa/review-scripts/deterministic_review.yaml --project examples/sample_project --json
```

Review templates accelerate exploratory QA by generating ordinary scenario or live-script files that run through existing commands. They do not replace final human review, do not auto-approve baselines, and do not write `qa/baselines`.

Artifact review workflow:

```bash
godot-qa artifact summarize .qa/artifacts/sample_main_menu_start/latest --json
godot-qa artifact summarize .qa/artifacts/sample_main_menu_start/latest --markdown-out .qa/artifacts/sample_main_menu_start/latest/review_summary.md --json
godot-qa artifact summarize .qa/artifacts/bundles/sample_regression_gate/latest --expect scenario_results --json
```

Agents should run `artifact summarize` before manually reading evidence folders. The default `artifact_summary.json` is the review index for scenario, live, bundle, and manual artifact roots; it lists category arrays, warnings, failures, and referenced paths without duplicating screenshots, audio, traces, or other binary artifacts.

Troubleshooting stale binaries:

`godot-qa doctor --json` reports duplicate `godot-qa` executables under `data.binary_surface` and warning entries under `data.warnings`. These warnings are informational by default; use `godot-qa doctor --json --strict-binary-surface` when CI or an agent environment should fail on an incompatible duplicate command surface. If `first_path_binary` points at an unintended binary, explicitly remediate the environment outside `doctor`: reinstall or remove stale `~/.cargo/bin/godot-qa`, update `PATH` so the intended unpacked release archive directory is first or remove older archive directories, and avoid placing repo-local `target/debug` or `target/release` binaries before the intended install. `doctor` never modifies `PATH`, deletes binaries, or runs shell snippets while probing duplicates.

## Using It In Your Own Project

1. Copy [`addons/godot_qa`](../addons/godot_qa) into your Godot project's `addons/` directory.
2. Enable the `Godot QA` plugin in the Godot editor.
3. Turn QA mode on only for test runs, using `--qa`, related `--qa-*` flags, or a test-only `godot_qa/enabled=true` setting.
4. Add scenarios under `qa/scenarios/`.
5. If you want explicit project-owned scene/script smoke tests, add `qa/tests/manifest.json` and referenced `scene` / `script` entries.
6. Run:

   ```bash
   godot-qa doctor --project /path/to/your/project --json
   godot-qa check --project /path/to/your/project --script res://src/main.gd --json
   godot-qa scenario list --project /path/to/your/project --json
   godot-qa check --project /path/to/your/project --tests --json
   ```

`check --script` is a quick Godot-backed GDScript load/compile check. It does not instantiate the script, does not require `run_qa_test()`, and is separate from manifest-backed `check --tests` script entries.

Manifest tags are optional, but they let you run a focused subset without changing the full-manifest default:

```json
{
  "version": 1,
  "tests": [
    {
      "id": "main_menu_startup",
      "kind": "scene",
      "target": "res://ui/main_menu.tscn",
      "tags": ["smoke", "ui"]
    },
    {
      "id": "sample_script_test",
      "kind": "script",
      "target": "res://qa/tests/script_smoke.gd",
      "tags": ["smoke", "script"]
    }
  ],
  "profiles": {
    "smoke": {
      "tags": ["smoke"]
    },
    "full": {
      "tests": ["main_menu_startup", "sample_script_test"]
    }
  }
}
```

```bash
godot-qa check --project /path/to/your/project --tests --test sample_script_test --json
godot-qa check --project /path/to/your/project --tests --tag smoke --json
godot-qa check --project /path/to/your/project --tests --profile smoke --json
godot-qa check --project /path/to/your/project --tests --batch --tag smoke --json
godot-qa check --project /path/to/your/project --tests --json
```

Batching is opt-in and never the default. `check --tests --batch` keeps normal selection filters such as `--tag smoke`, but runs selected manifest entries in one Godot process for speed. JSON output reports `execution_mode` so automation can distinguish batch from isolated execution. Manifest entries that require process isolation can set `"batch": false`; batch mode rejects those selected entries instead of silently running them.

For UI targeting, prefer stable `qa_id` metadata or groups over fragile node paths. See [`docs/qa-ids.md`](qa-ids.md) for efficient QA anchors, registered state/probe providers, fixture drivers, legacy `get_qa_*_state()` / `qa_*_probe()` methods, and when debug labels should stay diagnostic instead of becoming automation targets.

### Project Launch Prefix

Projects that must run Godot through another command can declare a generic argv prefix in `qa/config.yaml`:

```yaml
godot:
  launch_prefix:
    - some-wrapper
  windowed_launch_prefix:
    - xvfb-run
    - -a
```

`godot.launch_prefix` is a generic argv prefix applied to ALL CLI-owned Godot launches as `<prefix> <godot-binary> <godot-args>`. `godot.windowed_launch_prefix` is an ADDITIONAL prefix applied only for windowed launches (screenshots and live capture), after `launch_prefix`. Neither is specific to `xvfb-run`; use any wrapper command or script your project needs.
Use `qa/config.local.yaml` for uncommitted local overrides. Local config overlays keys from `qa/config.yaml`; `godot.launch_prefix: []` clears an inherited prefix.

## Install Notes

Supported user install paths:

- Download the published archive for your platform, extract it, and place `godot-qa` on `PATH`.
- Build from source in a repository checkout with `cargo install --path crates/godot-qa-cli`.

Published release archives contain:

- the `godot-qa` executable (`godot-qa.exe` on Windows)
- `README.md`
- `LICENSE`
- the matching `addons/godot_qa/` addon, documentation, and skill sources

Published targets:

- `x86_64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`

Every release also publishes a SHA256 checksum file covering all archives.

For in-repo development or smoke verification, you can run the Rust entrypoint directly:

- `cargo run --bin godot-qa -- --help`
- `cargo run --bin godot-qa -- doctor --project examples/sample_project --json`
- `cargo run --bin godot-qa -- import --project examples/sample_project --json`
- `cargo run --bin godot-qa -- check --project examples/sample_project --scene res://ui/main_menu.tscn --json`
- `cargo run --bin godot-qa -- check --project examples/sample_project --tests --json`
- `cargo run --bin godot-qa -- session start --project examples/sample_project --json`
- `cargo run --bin godot-qa -- tree --project examples/sample_project --json`
- `cargo run --bin godot-qa -- mcp --help`

## AI Skill

This repo also ships a bundled AI skill package at `skills/godot-qa/`.

`SKILL.md` provides the core QA workflow and routes agents to focused references for scenario authoring, bundles, visual evidence, runtime instrumentation, diagnostics, and live workflows. Load only the reference needed for the current task. The native installer embeds the complete package, so these references remain available in consuming projects without this repository checkout. Exact command options come from the installed CLI help; upstream contract docs are optional deeper references.

- For external skill installers that understand repo-hosted skills, this repo is compatible with commands such as `npx skills add <repo> --skill godot-qa`.
- For native repo-local installation into a Codex-compatible project scope, run `godot-qa install-skills --project .`.
- The native installer copies the bundled skill into `.agents/skills/godot-qa`, is project-scoped and Codex-first in v1, and does not shell out to Node, npm, or `npx skills add`.

## Current Status

`godot-qa` is alpha software: usable for evaluation and early adoption, but still evolving.

Current limitations worth knowing up front:

- Inspection surfaces stay intentionally bounded:
  - `node inspect` returns one node plus one-hop context, not arbitrary property browsing
  - `errors` exposes the structured QA runtime-error buffer, not generic engine-log scraping
- In some `--headless` environments, Godot may not expose real viewport pixel data, so screenshot artifacts can fall back to diagnostic PNGs while JSON runtime data remains real
- The built-in YAML loader covers the repository's sample scenarios, but broader YAML features may still require extra local tooling during internal development
- The project is still repo-oriented, so addon installation is currently a manual copy into `addons/`
- Recorder review remains manual; there is no current `scenario approve` command in the public contract

## Learn More

- [Sample project walkthrough](../examples/sample_project/README.md)
- [CLI command contract and current public command inventory](cli-spec.md)
- [Scenario format](scenario-format.md)
- [Architecture](architecture.md)
- [Test strategy](test-strategy.md)
- [Security model](security-model.md)

## For Contributors

See [CONTRIBUTING.md](../CONTRIBUTING.md).

## License

MIT.

## Scenario bundles

`godot-qa` also supports declarative scenario bundles under `qa/gates` for reusable regression gates.
Bundles group required/optional scenario names, compose reusable gates with `required_bundles`, and record recommended manual checks; they are not a CI scripting language.
Direct `scenario run` remains the source behavior for each child scenario; scenario-owned `godot_args` and `project_args` are both preserved exactly as direct scenario runs preserve them.
Bundle summary artifacts add expansion metadata such as `source_bundle`, `bundle_chain`, `run_root`, `godot_args`, launch display metadata, and pass/fail status.
Duplicate required scenarios, missing included bundles, unsafe references, and recursive-bundle cycles fail before child execution; missing optional scenarios warn by default and fail under `--strict`.

Sample commands:

```bash
godot-qa scenario bundle list --project examples/sample_project --json
godot-qa scenario bundle run sample_regression_gate --project examples/sample_project --json
godot-qa scenario bundle run composed_regression_gate --project examples/sample_project --json
godot-qa scenario bundle run screenshot_heavy_regression_gate --project examples/sample_project --json
```

## Verify a texture and sound asset trial

An asset trial binds source media into an existing saved scene temporarily,
runs one deterministic scenario, and leaves passing changes ready for an
explicit keep or discard. The sample manifest binds a PNG to a coin sprite and
a PCM16 WAV to its persistent pickup sound player. The gameplay scenario starts
a parent scene that instances the edited scene, captures the visible sprite,
then schedules the pickup before capturing SFX playback.

The opt-in Rust integration acceptance covers PNG-only, WAV-only, and combined
trials; status, discard, keep plus fresh replay; and conflict retry recovery.
It creates disposable copies of the complete sample and
runs real Godot imports, rendered captures, and non-silent audio capture. On a
local Linux desktop with Godot, Gamescope, and an audio server available, run
the rendered acceptance in a headless Gamescope display:

```bash
XMODIFIERS=@im=none /usr/games/gamescope --backend headless \
  --expose-wayland -W 1920 -H 1080 -- env XMODIFIERS=@im=none \
  GODOT_QA_ASSET_TRIAL_ACCEPTANCE=1 cargo test -p godot-qa-cli \
  --test asset_trial_cli real_asset_trial_lifecycle_acceptance \
  -- --exact --nocapture
```

CI runs this acceptance in a private Xvfb display only when
`asset_trial_acceptance` is enabled on the manual workflow dispatch. Local
rendered acceptance uses headless Gamescope as shown above. The normal quality
matrix runs the workspace tests on both Linux and Windows, including the
platform-native filesystem coverage.

Run the trial only against a disposable copy because `asset try` changes the
selected project's scene until it is kept or discarded. For example:

```bash
cp -a examples/sample_project /tmp/godot-qa-asset-trial
godot-qa asset try qa/asset-trials/coin.yaml --project /tmp/godot-qa-asset-trial --json
godot-qa asset status --project /tmp/godot-qa-asset-trial --json
godot-qa asset discard <trial-id> --project /tmp/godot-qa-asset-trial --json
```

`status` includes the generated trial ID and before/after evidence paths. After
reviewing a passing candidate, use `asset keep <trial-id>` instead of discard.
The sample project includes a pre-existing `.gdignore` under `.qa/artifacts/`
so its evidence cannot be imported as game content. The standalone scenario and
regression bundle can be run without applying a trial:

```bash
godot-qa scenario run asset_coin_pickup --project examples/sample_project --json
godot-qa scenario bundle run asset_trials --project examples/sample_project --json
```

## Android live sessions

Android is a first-class target for authenticated live sessions:

```bash
godot-qa session start \
  --target android \
  --preset "Android QA" \
  --device <device-serial> \
  --json
```

The CLI exports a debug APK, installs it with ADB, provisions the one-shot session
bootstrap through the app's private storage, forwards only that session's bridge
port, and pulls device-generated artifacts into the normal host `.qa/artifacts`
tree. `--device` may be omitted when exactly one authorized device is connected;
`--package` overrides preset package discovery for custom Gradle builds. ADB is
resolved from `ADB`, `ANDROID_SDK_ROOT`, `ANDROID_HOME`, then `PATH`.

The debug-APK export is cached under `.qa/cache/android/`, so a second start on an
unchanged project reuses the exported APK instead of re-exporting; pass
`--no-apk-cache` to force a fresh export (which still refreshes the cache entry).

A single scenario can also run one-shot on a device — deploy, run, and tear down:

```bash
godot-qa scenario run sample_main_menu_start \
  --target android \
  --preset "Android QA" \
  --json
```

Android sessions support live inspection, input, assertions, screenshots,
snapshots, audio, profiling, traces, pause/step, live edit
(`edit-live set/call/diff/export-patch`), packaged-scene reload, and
one-shot `scenario run`. Release/AAB deployment, Android one-shot `check`,
`scenario bundle`, `scenario sweep`, or `scenario profile` execution, watch/hot
reload, warm pools, headless mode, fixed/fastest pacing, host-controlled viewport
resizing, and `--no-audio` are intentionally outside the initial scope. See [the
CLI contract](cli-spec.md#android-live-sessions) and [security
model](security-model.md#android-live-session-boundary).
