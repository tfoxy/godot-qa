# Repository Guidelines for AI Agents

This repository is intended to be worked on by AI coding agents. Follow these rules strictly.

## Mission

Build `godot-qa`: a Godot runtime QA addon plus CLI that lets humans and AI agents validate Godot UI/gameplay work through reproducible scenarios, live scene inspection, input simulation, screenshots, layout audits, logs, and structured assertions.

## Compatibility requirements

- Godot projects may live below a repository root. Resolve config, artifacts, and `--project` against the selected Godot project.
- CLI and addon versions must stay aligned, including consumers that copy or link the addon during development.
- Preserve warm and warm-strict behavior and bounded parallel bundle execution. Warm-strict is a supported everyday workflow.
- Parallel bundle children are separate Godot processes sharing one `user://` directory. Keep per-process scratch paths and session tokens isolated.

## Operating principles

- Prefer small, reviewable changes. Do not implement unrelated functionality unless the current task requires it. Keep public APIs small and documented.
- Preserve user changes. Never use `git reset`, `git checkout --`, `git clean`, or similar destructive commands unless the user explicitly asks.
- Do not use git worktrees unless the user explicitly requests them.
- Make failures actionable. Every validation failure should identify the command, target, expected state, actual state, and artifact paths when available.
- Keep the CLI as the source of truth. MCP, editor integrations, and CI should call the CLI or share its internal library; do not fork behavior.
- The Godot addon is the source of truth for runtime state. Do not fake live scene tree, UI rects, focus state, or screenshot data outside Godot.
- Use structured JSON for machine-readable output. Human output may be friendly, but every command used by an agent should support `--json`.
- Avoid raw `.tscn` editing for scene mutation. Prefer Godot APIs, generated scripts, or structured patches applied by Godot.
- Do not auto-approve screenshot baselines. Generate diffs and reports; baseline approval is a separate, explicit human action.
- Treat live bridge commands as serialized per active session. Do not run long live commands such as `trace record`, `event trace`, `audio capture`, `surface audit`, `session profile`, screenshot sequences, or live layout audits in parallel against the same session unless the command contract explicitly says it is parallel-safe.
- Never claim a change is complete unless all relevant verification commands pass or you clearly document why they cannot be run.

## Coding conventions

### CLI implementation and current guidance

- The shipped CLI/library is the Rust Cargo workspace. Use Rust-oriented verification and implementation assumptions.
- Keep the current CLI dependency-light and preserve stable machine-readable result contracts.
- Keep process spawning, protocol I/O, scenario parsing, assertions, and output formatting cleanly separated by subsystem.
- Commands must exit non-zero on validation failures unless the command is purely informational.
- Files over roughly 2,500 lines should either have a clear subsystem-level reason to stay together or be split behind a facade. Prefer small implementation modules that preserve existing public command and JSON contracts.

### GDScript addon

- Target Godot 4.x. Avoid engine-version-specific APIs unless documented in `docs/implementation-decisions.md`.
- Runtime QA code must be inactive unless test mode is explicitly enabled.
- Use stable `qa_id` metadata or groups for UI targets. Do not rely only on fragile node paths.
- Return JSON-compatible dictionaries and arrays only.
- Wait for a rendered frame before taking screenshots.
- Avoid `eval`-style script execution in the bridge. If unsafe execution is introduced later, it must be explicitly opt-in and clearly marked.
- Keep public addon script paths stable. Large scripts may use facade files at the existing paths with implementation helpers behind them, but command strings and response dictionaries must not change during maintainability refactors.

### Docs

- Update docs when behavior changes.
- Keep contract docs in sync with implementation: `docs/cli-spec.md`, `docs/scenario-format.md`, and `docs/runtime-bridge-protocol.md` are pinned by `crates/godot-qa-cli/tests/repo_contract.rs`, so a command/protocol/format change that isn't reflected in these docs will fail that test suite.
- Add examples whenever a new scenario/protocol/artifact format is introduced.
- Use Conventional Commits (`feat:`, `fix:`, `perf:`, `docs:`, `refactor:`, `test:`, `ci:`, `build:`, `chore:`). Follow [CONTRIBUTING.md](CONTRIBUTING.md) for release-impact rules.

## Required verification pattern

For any change, run the strongest available checks. Changes confined to CLI/library logic can rely on `cargo test`; changes that touch the Godot addon or runtime bridge should also run Godot-based smoke checks.

Preferred order:

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
godot-qa doctor --json
godot-qa scenario list --json
godot-qa scenario run <scenario> --json
```

Godot scenario commands below are additional acceptance validation for runtime behavior; they do not replace the Rust quality gate above.

When Godot integration exists, also run:

```bash
godot --headless --path <sample_project> --import
godot-qa check --project <sample_project> --json
```

## Definition of done for UI/runtime features

A UI/runtime feature is not done unless it has:

- CLI command contract.
- Godot-side implementation.
- Machine-readable JSON output.
- At least one example scenario or fixture.
- Automated tests where practical.
- Documentation update.
- Clear failure behavior.

## Security rules

- Runtime bridge must bind to `127.0.0.1` by default.
- Runtime bridge must require a session token for live commands.
- Do not expose filesystem write commands through the runtime bridge except controlled artifact output paths.
- Do not include the QA bridge in release exports by default.
- Treat live edit commands as temporary unless explicitly exported to a patch.

## Agent completion response

When finishing a change, summarize:

```text
Implemented:
- ...

Verification:
- command: result
- command: result

Changed docs:
- ...

Known limitations:
- ...
```
