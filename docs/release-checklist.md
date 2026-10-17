# Building and releasing godot-qa

Release bundles target **Linux x64** (`x86_64-unknown-linux-gnu`) and **Windows
x64** (`x86_64-pc-windows-msvc`). Linux is built on Ubuntu 22.04; use a compatible
glibc 2.35 or newer environment. Windows is built and tested on Windows Server
2022. The checked-in Rust toolchain and Godot 4.5.2 are pinned in CI.

## Local development

Run `cargo xtask prepare-sample` after cloning or changing addon files. It copies
the canonical addon into the ignored sample addon directory and creates a native
process fixture for isolated process-outcome tests. It preserves local configuration
and does not wrap ordinary sample runs. It needs neither symlinks nor Windows
administrator privileges. Linux CI explicitly uses `xvfb-run` for windowed tests;
Windows uses its desktop. Set `GODOT_BIN` to the intended Godot executable when needed.

Run `cargo xtask install-hooks` once to enable the Conventional Commit subject
check. Supported types: `feat`, `fix`, `perf`, `docs`, `test`, `refactor`, `build`,
`ci`, `chore`, `revert`, and `style`, with an optional `(scope)` and optional `!`.
Use `BREAKING CHANGE: explanation` in the body for breaking changes.

`cargo xtask check` prepares and imports the sample, runs `cargo test`, formatting
and Clippy, builds the CLI, then runs doctor, project checks, scenarios, and a
live start/input/snapshot/stop smoke test. Do not share an active sample session
with another test run.

## Prepare a release locally

```sh
cargo xtask release --dry-run
cargo xtask release prepare
# Review the version/changelog commit before explicitly creating its tag:
cargo xtask release tag
```

These commands never push, create a pull request, or publish a release.
`prepare` requires a clean worktree, runs the validation suite, changes versions
and changelog, repeats validation, and commits those files only after success.
A failure leaves inspectable uncommitted changes and creates no release commit.
`tag` requires the matching prepared release commit and creates an annotated
local tag. Pushing that tag is a separate maintainer action.

Bump `[workspace.package]` version and `addons/godot_qa/plugin.cfg` version together;
the tool also updates workspace package versions in Cargo.lock. After the initial
`0.1.0`, `fix` and `perf` cause patch releases; `feat` causes a minor
release. Breaking changes cause minor releases before 1.0 and major releases
thereafter. Documentation and maintenance alone do not trigger a version bump.
Graduation to 1.0 is explicit: `cargo xtask release prepare --version 1.0.0` (preview
with `cargo xtask release --dry-run --version 1.0.0`).

The first public release is exactly `0.1.0`, with a fresh public changelog; old
pre-publication history is excluded. Subsequent changelogs use the pinned tool:

```sh
cargo install git-cliff --version 2.10.1 --locked
```

The release tool rejects another git-cliff version and reads only commits after
the latest reachable stable version tag. Commit messages must follow the hook's
Conventional Commit rules. `cliff.toml` controls the public release notes.

## Native packages and GitHub Actions

```sh
cargo xtask package x86_64-unknown-linux-gnu
# Run on Windows:
cargo xtask package x86_64-pc-windows-msvc
```

Both commands build with the optimized release profile (`opt-level=3`, thin LTO,
one codegen unit, stripped symbols), preserving a generic target CPU. Archives
contain the executable, matching `addons/godot_qa`, bundled skill source, `docs`,
README, CONTRIBUTING, AGENTS, CHANGELOG, LICENSE, and a runnable sample project.
Only tracked source files enter the archive; local overrides and ignored artifacts
are excluded. The sample addon is materialized from the tracked canonical addon. Godot itself remains a separate installation.

For the published Linux compatibility baseline, build on Ubuntu 22.04 as CI does.
A local package built on a newer distribution can require a newer glibc; portable
CPU settings do not lower that system-library requirement.

The package check extracts the finished archive into a path containing spaces and
Unicode, imports its bundled sample, installs the embedded skill from an unrelated
working directory, and runs manifest checks, recording, cold and warm-strict bundles, focus inspection,
screenshots, audio export, hot reload, trace, profile, and session cleanup with the extracted executable. CI tests on both
native platforms, checks formatting and Clippy, builds both archives, and retains
artifacts for review. Manual dispatch produces artifacts only. A pushed `v*` tag
additionally verifies tag/Cargo/addon/changelog versions before publishing both
bundles. Only the final publishing job has `contents: write`; no publishing
secrets are required beyond GitHub's job token.

Publish one SHA256 checksum file (`SHA256SUMS`) covering every shipped archive.
The workflow verifies those checksums before creating the GitHub Release. No tag
or release is published by local tooling.

## Acceptance review

- Run `godot-qa doctor --project examples/sample_project --json --strict-binary-surface` using the intended installed binary.
- Inspect `data.binary_surface`. Resolve any stale `.cargo/bin`, release archive, or repo-local `target/debug` or `target/release` shadowing warning. Doctor
  does not modify `PATH` or delete files; adjust the installation yourself.
- Confirm `data.cli.version` and `data.addon.version` agree.
- Review native Linux and Windows CI results, including process cleanup, session
  ownership, warm-strict runs, paths containing spaces, and failure exit codes.
- Confirm archives contain no `.qa`, `.godot`, local configuration, credentials,
  or build-machine resource paths.
- Keep screenshot baseline approval a separate human action.
