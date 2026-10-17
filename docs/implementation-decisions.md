# Implementation Decisions

This file records decisions agents should not relitigate without a good reason.

## Decision 001: CLI first, shared MCP behavior

The stable automation layer is the `godot-qa` CLI. The optional MCP server calls the same library behavior.

## Decision 002: Rust CLI and GDScript runtime

The CLI/library uses Rust for process management, transport, orchestration, and portable distribution. The Godot addon uses GDScript for engine-owned runtime state.

## Decision 003: Runtime truth comes from Godot

The CLI must not infer scene tree/UI state from `.tscn` files. Runtime scene state, focus owner, and screenshots come from Godot-side code.

## Decision 004: `qa_id` is the preferred target locator

Stable test IDs should use node metadata or groups. Node paths are supported but not preferred.

## Decision 005: Screenshot baselines require human approval

Agents may produce candidate screenshots and diffs, but baseline approval is explicit.

## Decision 006: Plugin enables autoload, runtime stays opt-in

Enabling the addon plugin registers the `GodotQaBridge` autoload singleton. Runtime QA behavior still stays dormant unless `--qa`, a `--qa-*` flag, or the `godot_qa/enabled` project setting explicitly enables it.

Projects should use `GodotQaBridge.is_runtime_qa_active()` when gating expensive runtime QA state. Editor-only tools such as the Godot QA Profiler may write artifacts without activating that flag.

## Decision 007: Screenshot diff uses RGBA different-ratio thresholding

Screenshot comparison converts images to RGBA, requires exact size match for a passing result, counts a pixel as different when any RGBA channel differs, and defines `different_ratio = different_pixels / total_pixels`.

`threshold` is the maximum allowed `different_ratio`. The default is `0.01`, so `0.0` means exact match and `0.01` allows up to 1% differing pixels.

## Decision 008: Portable release distribution

Supported end-user install paths are published release archives and Cargo source builds. Release targets are `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-msvc`. Archives contain the optimized executable, matching addon, documentation, skill sources, README and LICENSE; every release publishes one SHA256 checksum file covering all shipped archives.

Default CI and release verification exercise the Rust binary. Public command names, flags, JSON envelopes, exit codes, artifact layout, and runtime protocol payloads are compatibility contracts. Linux/Gamescope presented-frame capture is an explicitly platform-specific capability; ordinary desktop operations target both Linux and Windows.

## Decision 009: Project launch wrappers are argv prefixes

Projects that need to run Godot through another command declare `godot.launch_prefix` in `qa/config.yaml`. Local-only overrides may be declared in ignored `qa/config.local.yaml`.

The prefix and default launch args are YAML arrays of argv entries, not shell strings. This keeps behavior deterministic across agent, local, and CI invocations and avoids adding shell parsing or quoting rules to the CLI. The prefix setting is intentionally generic: examples may use `xvfb-run`, but the CLI treats it as an arbitrary wrapper command and applies it as `launch_prefix + selected Godot binary + generated/config/scenario Godot args`.

`godot.args` supplies project-wide Godot engine args before the `--` separator. `godot.project_args` supplies project/game args after generated QA bridge args. Scenario or command-specific args are additive and come after config defaults on their side of the separator. `godot.launch_mode` may be `auto`, `headless`, or `windowed`; explicit CLI launch-mode flags override config, and config overrides automatic mode selection. When local config exists, local arrays replace base arrays, empty local arrays clear base defaults, and local `godot.launch_mode` replaces the base mode.

S43 keeps that launch contract intact but carves out one diagnostic-only exception: version probes may retry direct `godot --version` after a prefixed probe fails. The retry is not a general launch fallback, and it does not bypass an explicitly invalid Godot binary path. Successful fallback is reported as structured `data.godot.version_probe` metadata so agents can see which command produced the version.

## Decision 010: Screenshot evidence requires a real display backend

Screenshot-backed scenarios and visual assertions prove rendered pixels, so the CLI must not silently convert them into synthetic or non-visual checks in headless environments.

For S43, automatic virtual display use is limited to safely detected wrappers that are actually available on the host. The CLI may select such a wrapper for screenshot-producing launches and records the selection under `launch_display`. If no host display or safe wrapper is available, screenshot-backed runs fail before runtime execution with `display_backend_unavailable`, while non-screenshot scenarios may remain headless.

## Decision 011: Physics overlay artifacts are offline review evidence

S67 collider overlays are composed as artifacts, not as a game-facing debug HUD. Godot owns runtime metadata collection and screen projection for supported 2D collision evidence; the CLI owns copying the base screenshot, rendering the overlay PNG, writing `.overlay.json`, and applying warning/strict semantics. The runtime must not add visible overlay nodes or mutate gameplay state while collecting evidence.

Current Godot 4.x limitations:

- Shape support is intentionally 2D-first and limited to shapes the runtime collector can describe as screen-space points or AABBs. Unknown or unsupported shape kinds may be reported as `unknown` with bounded metadata instead of being visually approximated beyond the runtime evidence.
- Screen projection is based on active runtime canvas/viewport state at collection time. Offscreen targets can produce `physics_overlay.target_offscreen`; the CLI must not repair that by reading `.tscn` files or synthetic coordinates.
- Contact points and normals are included only when live physics data is available and `--include-contacts` is requested. Saved snapshot review can render contacts only if they were captured in the saved `physics_overlays` metadata.
- Project visual footprints are optional, read-only QA provider data. Without an approved provider such as `get_qa_visual_footprint_state()` or `get_qa_collision_visual_state()`, visual-footprint comparison warnings are omitted rather than guessed from sprites, textures, names, or project-specific semantics.
- Shape collection is bounded by `max_shapes`; truncation is deterministic and reported as `physics_overlay.unbounded_shape_count_truncated`.

## Decision 012: Scenario template vars are file-static; no `--var` CLI flag

Scenario templating (`extends` + `vars` + `abstract`) resolves entirely from files at load time. There is intentionally no run-time `--var name=value` CLI flag to override vars per invocation.

The reason is identity: artifacts, bundles, and lint all key on a static scenario name. Artifacts are written under `.qa/artifacts/<name>/<run-id>/`, bundles resolve scenarios by name, and lint's duplicate-name and reference indexes assume one name maps to one document. A run-time `--var` override would make a single scenario name mean different documents on different runs, so a screenshot baseline, a bundle entry, or a lint finding could no longer be tied to a reproducible document. Authors who need a different tuning create another concrete child (a distinct name) instead of mutating one at run time. If a future spec needs parameterized runs, it must first define how artifact identity and baseline approval stay reproducible.

## Engine and transport baseline

Release CI uses Godot 4.5.2. Other Godot 4.x versions require validation before expanding the supported baseline. Live sessions use token-authenticated JSON lines over loopback TCP.

## Decision 013: Android live sessions reuse the authenticated bridge

Android support exports and deploys a debuggable APK, provisions a one-shot
bootstrap through `run-as` stdin, and reaches the existing loopback-only bridge
through one exact ADB forward. It does not expose the bridge on a device network
interface and does not introduce an Android-specific command protocol.

Device paths are derived from the session ID and treated as untrusted when they
return to the host. The initial Android scope deliberately favors correctness
over deployment speed: there is no warm pool, watch/hot reload, or release/AAB
deployment.

## Decision 014: Android debug-APK cache is conservative and self-healing

`session start --target android` caches the debug-APK export under
`.qa/cache/android/<key>/` so an unchanged project skips the export on the next
start. The key is a SHA-256 (a dependency-free implementation in `godot-qa-core`,
following the crc32/zlib precedent) of the Godot binary path and probed version,
the preset name, the resolved package, and a sorted manifest of every project
file's relative path, size, and mtime-in-nanoseconds.

The manifest uses size+mtime rather than a content hash on purpose: the only
failure direction is a *false miss* (any write changes the key and forces a fresh
export), never a false hit that would deploy a stale APK. Cost stays O(file
count). The walk mirrors the Godot exporter's exclusions — hidden `.` directories
(`.git`, `.godot`, `.import`, `.qa`), any directory containing a `.gdignore`, and
the artifacts directory when it resolves inside the project root — and follows
symlinks (with a canonicalized visited set and depth cap) so a symlinked addon
directory is still hashed.

The cache never fails a session. A probe or manifest failure records
`apk_cache: {status: "bypassed"}` and exports without caching; a corrupt,
key-mismatched, or resized entry is removed on lookup and treated as a miss; a
store error is a non-fatal deployment-log warning. Eviction keeps the newest four
entries by last-use. `--no-apk-cache` is docker-style: it bypasses the read but
the fresh export still refreshes the entry.

Residual risks: an mtime-granularity race where a same-size file is rewritten
within the filesystem's mtime resolution could serve a stale hit, and an in-place
export-template swap that keeps an identical version string is invisible to the
key. Both are narrow; consumers with large non-Godot directories should
`.gdignore` them to keep the manifest walk cheap.

## Decision 015: Android one-shot scenario execution is bridge-driven

`scenario run --target android` runs a single scenario on a device by reusing the
`session start --target android` deploy stages to bring up an ephemeral
scenario-mode session, then driving that scenario through the authenticated
`scenario.run` bridge command — the same command the desktop warm path uses. No
Android-specific scenario protocol is introduced, and the shared
`finalize_runtime_run` produces the verdict, so cold, warm, and Android one-shot
runs return an identical result envelope.

The engine reads every output path from request.json, so a run-scoped artifact
mapper rewrites the host request's artifact map to the session's private device
root (`user://godot_qa/live/<session-id>/run`) before the request is pushed. The
engine then writes result.json and evidence into app-private storage; the CLI
pulls them back with the existing binary-safe ADB transport and substitutes the
echoed device artifact map for the host layout so host-only files (logs.txt,
meta.json) are never pulled.

Device pacing is real, so the request's `run.frame_budget` is stripped and the
scenario timeout is enforced purely as a wall clock. A scenario preflight rejects
inputs that cannot be honored faithfully on device — process-launch `godot_args`,
an explicit `fixed_fps`, viewport/layout matrices, and host-viewport
`screenshot_match` baselines — with `android_scenario_unsupported`. Cleanup always
runs and a failed cleanup step fails the run. Android one-shot `check`,
`scenario bundle`/`sweep`/`profile`, warm pools, and hot reload/watch remain out
of scope.

## Decision 016: `project.godot` reads go through one stamped process cache

`project.godot` is read on several unrelated hot paths — frame-pacing resolution,
the Linux Wayland preference probe, live-session main-scene resolution, and
scenario main-scene resolution — so a warm bundle re-reads the same small file
once per scenario per lane. `godot_qa_core::project_file::read_project_godot_cached`
serves those reads from a process-wide cache keyed on the file's `(mtime, len)`
stamp, taken from a single `fs::metadata` call. Repeat readers share one `Arc<String>`
and pay a stat instead of a read; every caller keeps its existing signature and
parses the contents exactly as before.

Correctness is bounded by the stamp, not by trust: an out-of-band rewrite changes
mtime (and usually length) and invalidates the entry, which is what keeps
`diagnostics editor` honest — it mutates `project.godot` in-process for the editor
import and then restores the snapshot. When the stamp cannot be taken at all the
read falls through uncached, so a missing or unreadable file degrades to the
previous behavior rather than to a stale answer. The residual risk is the same
mtime-granularity race as Decision 014: a same-length rewrite inside the
filesystem's mtime resolution could serve a stale hit.

The read is *not* skipped for headless launches. `DisplayLaunchMetadata` carries
`project_prefers_wayland` for headless runs too, and that field is emitted in
scenario-run, visual-preflight, and live-session result JSON, so gating the read
on `!headless` would flip a published value from `true` to `false` for any project
with `run/platforms/linuxbsd/prefer_wayland=true`. The cache removes the cost the
guard was chasing without changing the contract.

## Decision 017: The idle-session throttle is host-local, not `Engine.max_fps`

An idle live session used to spin its serve loop at the process frame rate. That is
cheap for a plain `--headless` session, where the engine's own frame delay caps an
undrawable window near 144 fps, but pooled warm sessions launch with
`--fixed-fps <n> --disable-vsync`, which removes that delay: a measured idle pooled
session burned 99% of one core and listed the file-transport request directory about
25,000 times per second while doing nothing.

The obvious fixes — `Engine.max_fps` or `OS.low_processor_usage_mode` — are off limits.
`Engine.max_fps` is batch-global engine state that `qa_scenario_executor.gd` snapshots
per test and guards with `runtime_contamination_detected`, so a host that changed it
would either be reported as contamination or, worse, silently redefine the frame budget
that scenario timing and `session.wait_frames` are measured in.
`OS.low_processor_usage_mode` has the same problem from the other direction: it changes
how long every frame takes for the project under test, not just for the QA host.

So the throttle lives entirely in `qa_live_session_host.gd` and is scoped to the
serve loop. The host counts in-flight `_handle_connection` / `_handle_file_request`
coroutines; only when that count is zero does it `OS.delay_msec(1)` before awaiting the
next frame and poll the file transport every 8th idle frame instead of every frame. A
command in flight is never delayed, which is what keeps the 5 s live-session request
timeout and the `LIVE_SESSION_WAIT_TIMEOUT_FPS = 30` frame budget intact. The counter
fails safe: a leaked increment can only disable the throttle — today's behavior — and
never throttles a live command.

The same idle path bounds `_processed_file_requests`, which previously grew one entry
per file request for the lifetime of a pooled session. Pruning runs only while nothing
is in flight, so an entry can only be dropped after its request file has been deleted
by the request that owned it, and a dropped key can never be re-listed.
