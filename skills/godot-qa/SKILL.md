---
name: godot-qa
description: Validate Godot runtime behavior with the godot-qa CLI and addon using scenarios, live inspection, screenshots, assertions, and artifact-backed debugging. Use for project QA setup and execution, not general Godot development without a QA task.
---

# Godot QA

Use the installed `godot-qa` CLI to gather evidence from the actual Godot runtime. Start with the workflow below; read only the reference relevant to the task. The skill supports both existing regression suites and targeted investigation of a running game.

## Orient and run

1. Find the selected project's `project.godot`; it may be below the repository root. In examples, `<project>` is that directory, `<scenario>` is a discovered scenario name, and scene paths and targets must be replaced with actual project values. Pass `--project` consistently so configuration, scenarios, and evidence belong to the intended project.
2. Read existing project QA instructions and `qa/config.yaml`, plus `qa/config.local.yaml` when present. Preserve configured launch behavior and regression defaults. Check readiness and discover saved checks:

   ```bash
   godot-qa doctor --project <project> --json
   godot-qa scenario list --project <project> --json
   ```

   Inspect warnings as well as failures. Resolve missing or mismatched CLI/addon installations before trusting results. For fresh checkouts or import problems, use `godot-qa import --project <project> --json`.
3. Run the narrowest existing scenario that exercises the requested behavior:

   ```bash
   godot-qa scenario run <scenario> --project <project> --json
   ```

   Use `check --scene res://...` for scene load checks, `check --script res://...` for script load/compile checks, or `check --tests` for the project's explicit `qa/tests/manifest.json`. These checks do not establish player-facing behavior on their own. For regression gates, discover `scenario bundle list` and read the bundle reference before changing execution flags.
4. If no suitable scenario exists, use live inspection for diagnosis or the scenario-authoring reference for a repeatable check. Exercise real input and assert observable outcomes; fixture setup must not substitute for the behavior under test.

Use `godot-qa --help` and `godot-qa <command> --help` for the installed version's exact options. Prefer `--json` for agent-consumed results; use command-specific help before assuming an unfamiliar option exists.

## Live inspection

Check `session status --project <project> --json` first. Reuse an appropriate active session; start one when needed. The following illustrates a new windowed session and a targeted interaction:

```bash
godot-qa session start --project <project> --json
godot-qa tree --project <project> --json
godot-qa node inspect qa:start_button --project <project> --json
godot-qa input click qa:start_button --project <project> --json
godot-qa snapshot --project <project> --name after_click --json
```

Discover targets from runtime state rather than assuming the example locator exists. Prefer stable `qa_id` metadata or groups; `qa:`, `path:`, and `name:` locators must resolve uniquely. Stop sessions you started when finished, using `session stop`; leave pre-existing sessions under their owner's control.

## Evidence and constraints

- Read the result's `ok`, failures, warnings, and artifact paths together with the exit status. Report the command, target, expected and actual state, and relevant evidence. A startup or bridge failure does not prove a gameplay failure.
- Evidence defaults to the selected project's `.qa/artifacts/`; follow returned paths when configured otherwise. Run `godot-qa artifact summarize <run-root> --json` before opening individual files. Inspect relevant images and structured data before making visual or behavioral claims.
- Godot is authoritative for live state, focus, layout, and pixels. Source files alone cannot establish runtime correctness. Screenshot evidence requires real rendered pixels; use a windowed session. Headless diagnostic images cannot validate appearance.
- Serialize live bridge commands within each session, including long captures and profiles. Use `session run-script` for sequenced reviews. Treat `session_bridge_busy` as contention, not game-state evidence.
- Preserve warm, warm-strict, and bounded parallel execution settings. Investigate failures instead of disabling strictness to obtain a pass.
- Baseline approval is an explicit human action. Review recorder suggestions manually. Keep live edits temporary unless intentionally exported and applied; inspection alone does not authorize source edits.
- Keep the bridge on localhost with session-token authentication. Do not print or share token values. QA runtime code must remain inactive outside test mode.

## Load details only when needed

| Task | Reference |
|---|---|
| Setup, checks, live input/scripts, reload, edits, session failures | [CLI workflows](references/cli-workflows.md) |
| Write scenarios, set up fixtures, review recordings, simulate controllers | [Scenario authoring](references/scenario-authoring.md) |
| Regression bundles, warm-strict failures, parallelism, selection, retries | [Bundle gates](references/bundle-gates.md) |
| Screenshots, layout, visual audits, artifact queries and export | [Visual and artifact review](references/visual-and-artifacts.md) |
| Add efficient QA anchors, state/probe providers, fixture adapters | [Runtime instrumentation](references/runtime-instrumentation.md) |
| Editor diagnostics, profiling, physics, traces, telemetry, audio | [Diagnostics and profiling](references/diagnostics-and-profiling.md) |

References contain task-specific examples, not a second command manual. Deeper `docs/` paths mentioned there belong to a matching godot-qa source checkout or distribution, not necessarily the consuming project. Use bundled guidance, existing project examples, and installed CLI help first; consult upstream contracts when exact format or API details are needed.
