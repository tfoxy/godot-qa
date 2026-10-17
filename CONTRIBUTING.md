# Contributing

Changes should preserve the documented CLI, JSON, scenario, and runtime bridge contracts. Keep work focused and describe the behavior changed and evidence used to verify it. [AGENTS.md](AGENTS.md) also applies to automated contributors.

## Development setup

Install Rust and Godot 4.5.2 (the CI baseline), then run from the repository root:

```sh
cargo xtask prepare-sample
godot --headless --path examples/sample_project --import
cargo build --locked --bin godot-qa
cargo test --locked
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
```

`prepare-sample` copies `addons/godot_qa` into the ignored sample addon directory on Linux and Windows. It also prepares a native process fixture under `.qa/tools/` for isolated process-outcome tests. It does not install a global launch wrapper or change local configuration. Repeat it after changing addon source. Never edit the generated copy. Source builds use ordinary Cargo commands; Make and mise are optional conveniences.

## Runtime verification

Put the built CLI and Godot on `PATH`, or use `cargo run --bin godot-qa --` in place of `godot-qa`:

```sh
godot --headless --path examples/sample_project --import
godot-qa doctor --project examples/sample_project --json
godot-qa scenario list --project examples/sample_project --json
godot-qa check --project examples/sample_project --scene res://ui/main_menu.tscn --json
godot-qa check --project examples/sample_project --tests --json
godot-qa scenario run sample_main_menu_start --project examples/sample_project --json
godot-qa scenario run sample_settings_layout --project examples/sample_project --json
```

Runtime/addon changes need relevant Godot checks in addition to the Rust gate. Screenshot tests need a display; Linux CI uses Xvfb. Headless diagnostic images do not prove rendered pixels. `screenshot assert-startup-presented` requires Linux/Gamescope. Keep commands against one live session serialized.

The sample contains intentional negative fixtures, including `runtime_error_probe` and `bad_settings_menu`; `check --all-scenes` is not a happy-path smoke test. Assert the expected failure instead of hiding exit status. Never approve or replace screenshot baselines automatically.

Use portable subprocess fixtures and paths with spaces when changing process handling. Check both supported native targets. Keep credentials, `.qa/` evidence, `.godot/` imports, and machine settings out of commits. Put local overrides in ignored `qa/config.local.yaml`.

## Documentation and commits

Update behavior documentation and the corresponding contract tests together. Keep onboarding in [README.md](README.md), detailed examples in [workflows](docs/workflows.md), and exact interfaces in the CLI, scenario, protocol, and artifact references.

Use Conventional Commits, for example `fix: preserve Windows paths in session cleanup`. `feat` introduces functionality; `fix` repairs behavior; `perf` improves performance. Use `docs`, `test`, `refactor`, `build`, `ci`, `chore`, `revert`, or `style` for other focused changes. Enable the local subject check with `cargo xtask install-hooks`. Mark incompatible changes with `!` or a `BREAKING CHANGE:` footer.

Before 1.0, `feat` and breaking changes select a minor release; `fix` and `perf` select a patch release. Other types do not trigger a version bump. Release preparation uses pinned git-cliff 2.10.1 to generate the changelog from these commits. The initial public release is 0.1.0; subsequent releases use commits after the latest stable version tag.

## Releases

Use `cargo xtask release --dry-run` to inspect the proposed version and release commit subject. `cargo xtask release prepare` requires a clean worktree, validates the repository, updates versions and the changelog, repeats validation, and creates the release commit only after success. Review that commit before tagging; a validation failure leaves inspectable uncommitted changes. `cargo xtask release tag` creates the local version tag. See the [release checklist](docs/release-checklist.md) for exact prerequisites and archive verification.

Publishing happens through GitHub Actions when a maintainer explicitly pushes a version tag. Local release commands do not push. No PR automation is required.
