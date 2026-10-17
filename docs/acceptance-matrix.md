# Acceptance Matrix

This table maps capabilities to repeatable acceptance checks. For the exact currently shipped CLI command surface, use [`docs/cli-spec.md`](cli-spec.md).

| Capability | Evidence |
|---|---|
| CLI imports and JSON result contracts | `cargo test`, `godot-qa doctor --json` |
| Project validation (`import`, bounded `check`) | `godot-qa import --project examples/sample_project --json`, `godot-qa check --project examples/sample_project --scene res://ui/main_menu.tscn --json`, `godot-qa check --project examples/sample_project --scene res://ui/fixtures/runtime_error_probe.tscn --json || true`, `godot-qa check --project examples/sample_project --all-scenes --json || true` |
| Validation contract guardrails and smoke verification | `cargo test`, `godot-qa import --project examples/sample_project --json`, `godot-qa check --project examples/sample_project --scene res://ui/main_menu.tscn --json`, `godot-qa check --project examples/sample_project --scene res://ui/fixtures/runtime_error_probe.tscn --json || true`, `godot-qa check --project examples/sample_project --all-scenes --json || true` |
| Optional manifest-backed `check --tests` expansion | `cargo test`, `godot-qa check --project examples/sample_project --tests --json` |
| Scenario listing | `godot-qa scenario list --json` |
| One-shot scenario execution | `godot-qa scenario run <name> --json` |
| Artifact directory with result/logs | `.qa/artifacts/<scenario>/<run>/result.json` |
| Screenshot/tree/UI/focus snapshots | snapshot artifact files |
| Action input and target click | scenario steps change runtime state |
| Assertions | scenario pass/fail with structured failures |
| Layout audit | offscreen/overlap/focus diagnostics |
| Live session | `session start`, `input tap`, `snapshot`, `session stop` |
| Screenshot diff | diff image and metrics |
| Live edit patches | exported patch JSON |
| Recorder | generated candidate scenario |
| CI/package | workflow and install test |
| MCP wrapper | `godot-qa mcp` tool surface |
| Sample project/hardening | `godot-qa scenario run sample_main_menu_start --project examples/sample_project --json`, `godot-qa scenario run sample_settings_layout --project examples/sample_project --json`, `godot-qa layout audit --project examples/sample_project --scene res://ui/fixtures/bad_settings_menu.tscn --json || true`, and the live-session smoke flow against `examples/sample_project` |
| Expanded live inspection | `godot-qa node inspect qa:start_button --project examples/sample_project --json`, `godot-qa physics inspect qa:physics_probe --project examples/sample_project --radius 300 --frames 2 --json`, `godot-qa trace record --project examples/sample_project --target qa:trace_probe --frames 60 --sample-every-frames 1 --physics --json`, `godot-qa focus graph --project examples/sample_project --json`, `godot-qa errors --project examples/sample_project --json` |
| Live screenshot surfaces | `godot-qa screenshot capture --project examples/sample_project --out .qa/artifacts/live-manual/main_menu.png --json`, `godot-qa screenshot compare --project examples/sample_project --live --baseline qa/baselines/live/main_menu.png --json` |
| Live layout audit | `godot-qa layout audit-current --project examples/sample_project --scenario sample_settings_layout --live --json` |
| Portable release packaging | `cargo test`, `godot-qa --help`, released archive contents plus SHA256 checksums, and the sample-project smoke block in `docs/release-checklist.md` |

## Android live-session acceptance

| Capability | Automated acceptance | Physical-device acceptance |
| --- | --- | --- |
| Android debug deployment | Fake export/ADB command order, preset and device validation, rollback, token redaction | Dedicated `Android QA` APK exports, installs, launches, and authenticates |
| Android live commands | Contract and runtime unit coverage, including an `edit-live` set-property round-trip through the Android transport; desktop regression suite | Status, tree/UI/focus/inspect, input, assertion, profile, logs, errors, and live edit (hide then restore a node on device) |
| Android artifacts | Binary pull, nested rewriting, traversal and partial-transfer failures | Screenshot and snapshot are host-local, parseable, and nonblank |
| Android cleanup | Exact forward/package/private-directory fake ADB assertions | App stopped, exact forward absent, bootstrap/session files absent, metadata cleared, package uninstalled |
| Android APK cache | Key stability/sensitivity, hit/miss/bypass, eviction, corrupted-cache degradation with fake Godot | Second start on an unchanged project logs `apk-cache: hit` and skips export |
| Android one-shot scenario | Fake bridge/ADB full post-deploy loop, option and scenario preflight tables, round-trip failure classification, and cleanup ordering | A sample scenario passes end-to-end on device with host-local result.json and evidence |
| Unsupported scope | `android_option_unsupported` and reload/watch contract tests | No release/AAB, one-shot check, hot reload, warm pool, headless, pacing, or viewport-resize claim |
