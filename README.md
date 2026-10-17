# godot-qa

**Turn Godot UI and gameplay checks into repeatable tests with evidence you can inspect.**

A menu can look correct in source and still fail at runtime: a button sits outside the viewport, controller focus gets stuck, or an interaction never changes the game state. `godot-qa` runs your real Godot project, simulates input, checks live state, and saves screenshots, logs, and structured results so you can reproduce failures and verify fixes.

Use it for local regression checks, CI, or AI agents that need feedback from the running game. It combines a Rust CLI with a small, opt-in Godot addon; the engine remains the source of truth.

## What you can do

- **Reproduce interactions:** save scenarios for menus, HUDs, controller input, and gameplay; combine them into regression bundles.
- **Inspect a running game:** query scene trees, UI geometry, focus, physics, and project-defined QA state; pause, step, and send input.
- **Review visual evidence:** capture screenshots, compare approved baselines, audit layouts, and inspect failures alongside logs and artifact paths.
- **Automate with confidence:** use stable `--json` results and nonzero failure exit codes from scripts, CI, the bundled AI skill, or the optional MCP server.
- **Measure behavior:** collect runtime profiles, traces, audio measurements, and bounded soak-test evidence.

This is alpha software. Screenshot baseline approval and judgments about appearance or playability remain human decisions.

## Install

Download the published archive for your platform from this repository's **Releases** page, extract it, and add its directory to `PATH`. Archives contain `godot-qa` (`godot-qa.exe` on Windows), the matching addon, documentation, skill sources, and the MIT license. Verify downloads against the release's `SHA256SUMS` file.

| Platform | Release target |
| --- | --- |
| Linux x64 | `x86_64-unknown-linux-gnu` |
| Windows x64 | `x86_64-pc-windows-msvc` |

Install Godot separately and make it available as `godot` on `PATH`, or pass `--godot-bin "path/to/Godot"`. CI uses **Godot 4.5.2**. Other Godot 4.x versions may work but are not the release validation baseline. Linux archives require glibc 2.35 or newer (the Ubuntu 22.04 build baseline); older Linux environments need a source build.

From a source checkout with Rust installed:

```sh
cargo install --path crates/godot-qa-cli
```

Visual tests need a working display/rendering backend. Linux CI can use Xvfb; Windows uses its native display. The specialized `screenshot assert-startup-presented` command requires Linux/Gamescope and is the documented platform-specific exception. Ordinary viewport screenshots and visual assertions work on both release platforms. See [platform requirements and workflows](docs/workflows.md).

## Try the sample

Release archives include a ready-to-run sample. Run the commands below from the extracted archive directory. For a source checkout, first run `cargo xtask prepare-sample` to copy the addon and prepare a native test fixture without filesystem symlinks.

```sh
godot-qa doctor --project examples/sample_project --json
godot-qa import --project examples/sample_project --json
godot-qa scenario list --project examples/sample_project --json
godot-qa scenario run sample_main_menu_start --project examples/sample_project --json
```

The scenario opens the sample menu, simulates input, and checks the resulting UI. Read the command's JSON for pass/fail details and artifact paths; generated evidence lives under the selected project's `.qa/` directory. The [sample walkthrough](examples/sample_project/README.md) includes layout, live-session, audio, and intentionally failing fixtures.

A scenario expresses the interaction and expected behavior directly:

```yaml
name: menu_smoke
scene: res://ui/main_menu.tscn
steps:
  - wait_frames: 5
  - click: qa:start_button
  - wait_frames: 10
  - snapshot: after_start
assert:
  - visible: qa:pause_menu
  - focused: qa:resume_button
  - no_errors: true
```

## Use it in your project

1. Copy `addons/godot_qa` from the release archive or source checkout into your Godot project's `addons/` directory. Keep CLI and addon versions aligned.
2. Enable **Godot QA** in Godot's Project Settings → Plugins. QA runtime behavior remains inactive during ordinary play; CLI test runs explicitly activate it.
3. Give important UI/gameplay nodes stable [`qa_id` metadata or groups](docs/qa-ids.md), then add scenarios under `qa/scenarios/` using the [scenario format](docs/scenario-format.md) and sample fixtures.
4. Run `godot-qa scenario run <scenario-name> --project "path/to/project" --json`.

`--project` points to the directory containing `project.godot`, which may be nested inside your repository. Keep `.qa/` artifacts and machine-specific `qa/config.local.yaml` out of version control. Exclude the QA addon from production exports; the [security model](docs/security-model.md) explains the loopback-only, token-authenticated bridge.

For AI-assisted work, run `godot-qa install-skills --project "path/to/project"` or configure the [MCP server](docs/mcp.md). Both use the same CLI behavior.

## Documentation

- [Workflows and capabilities](docs/workflows.md): live sessions, screenshots, bundles, Android, profiling, and review.
- [CLI reference](docs/cli-spec.md), [scenario format](docs/scenario-format.md), and [artifact format](docs/artifact-format.md).
- [Addon installation and integration](addons/godot_qa/README.md).
- [Architecture](docs/architecture.md), [contributing](CONTRIBUTING.md), and [release checklist](docs/release-checklist.md).

## License

[MIT](LICENSE).
