# Runtime instrumentation

Stable targets are important, but excessive QA instrumentation can make large scenes laggy. Prefer this pattern:

- Use `qa_id` metadata for durable anchors: important controls, player roots, match controllers, major containers, fixture anchors, and visible UI contracts.
- Avoid per-object QA IDs for every generated cell, projectile, salvage item, module fragment, or short-lived visual unless a scenario directly targets that rendered node.
- Keep metadata mostly static. Avoid high-frequency metadata or timed `set_meta()` mirroring for fast-changing values such as velocity, cooldowns, repair progress, telemetry counters, interaction distances, and placement state.
- Use runtime inspection/reflection first for engine-visible facts such as scene tree, UI rects, focus, visibility, transforms, and screenshots.
- Use small registered read-only providers for semantic aggregate state that Godot cannot infer from engine-visible facts. Prefer explicit provider IDs over adding public `get_qa_*` methods across gameplay classes.
- Use registered probe providers for bounded semantic queries such as surface, collision, walkability, or spatial classification.
- Use separate registered fixture drivers for deterministic mutation/setup, and keep them out of assertion and read-only state paths.
- Do not make QA adapter layers the production scene entry point. QA helpers should register adapters from normal scene code or QA-only fixtures without importing the rest of the project as a central wrapper.
- Legacy pull-based methods such as `get_qa_*_state()` and focused read-only probes such as `qa_surface_probe()` or `qa_*_probe()` remain supported for compatibility.
- Use `state_matches`, surface/proximity/temporal assertions, and visual assertions against durable anchors instead of parsing debug-label text when the debug label is not the player-facing contract.
- Use screenshot-backed visual assertions for rendered perception and keep debug overlays human-facing and optional. Hidden debug labels should not need frequent refreshes during normal play.
- Prefer aggregate state arrays from an anchor for many dynamic objects, with stable logical IDs inside the returned dictionaries, rather than one runtime node locator per object.

Use this reference when adding QA anchors or adapters to project code. Keep QA registration and setup inactive outside explicitly enabled test mode.

Inspect existing project adapters before adding new ones. Use `qa-state validate --help` for schema validation against a registered provider, a legacy method, or saved JSON; use `live-gate list` and `live-gate run` for project-owned validation profiles under `qa/live_gates/`. Pass the selected `--project` and `--json`.

For deterministic mutation examples, read [Scenario authoring](scenario-authoring.md). Registration API details are in `docs/qa-ids.md` and `docs/runtime-bridge-protocol.md` in a matching godot-qa source checkout or distribution; these are not assumed to exist in the consuming project.
