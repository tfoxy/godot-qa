# Scenario authoring

Use saved scenarios for reproducible behavior checks. Inspect existing files under the selected project's `qa/scenarios/` before creating a new one. JSON and the built-in YAML subset are supported; prefer simple mappings/lists over advanced YAML features.

## Create, lint, run

Generate a starter without launching Godot:

```bash
godot-qa scenario init menu_start --scene res://ui/main_menu.tscn --out qa/scenarios/menu_start.json --project <project> --json
```

Names must be unique within the scenario tree and safe for artifact paths: letters, numbers, `.`, `_`, and `-`. Required fields are `name`, `scene`, and `steps`. A minimal behavior check, after substituting the scene and locators discovered in the project:

```yaml
name: menu_start
scene: res://ui/main_menu.tscn
determinism:
  fixed_fps: 60
  seed: 123
  test_mode: true
snapshots:
  save_screenshot: false
steps:
  - wait_frames: 5
  - click: qa:start_button
  - wait_frames: 10
assert:
  - visible: qa:pause_menu
  - hidden: qa:main_menu_buttons
  - no_errors: true
```

Save as `qa/scenarios/menu_start.yaml` if authoring YAML instead of using the generated JSON; do not leave both files declaring the same name. Then:

```bash
godot-qa scenario lint --project <project> --scenario menu_start --json
godot-qa scenario run menu_start --project <project> --json
```

Lint is static validation, not execution. Warning-only findings exit zero unless `--strict` is requested; contract errors fail either way. For variable timing, use an existing project's `wait_until` pattern instead of arbitrary long sleeps. For rendered evidence, add named `snapshot` steps, enable `snapshots.save_screenshot`, and follow [Visual and artifact review](visual-and-artifacts.md).

Assertions should test the requested result: existence, visibility, focus, text, geometry, clean errors, or semantic state. Use `state_matches` and temporal/proximity/surface assertions when appropriate; inspect existing scenarios and matching format documentation for their exact shapes instead of guessing fields.

## Deterministic setup

Establish only the state needed to exercise the actual player-facing behavior. Fixture steps may remove slow travel or missing dependency blockers, but must not perform the behavior being tested. Available setup examples:

```json
{"teleport_node": {"target": "qa:player", "to": "qa:test_landing"}}
{"spawn_scene": {"scene": "res://fixtures/loot.tscn", "parent": "qa:arena", "qa_id": "test_loot", "at": "qa:loot_anchor"}}
{"set_property": {"target": "qa:team_1", "property": "scrap", "value": 10}}
{"fixture_call": {"driver": "match_setup", "action": "prepare_wave", "params": {"wave": 2}}}
{"call_method": {"target": "qa:match_controller", "method": "qa_prepare_wave_state", "args": [{"wave": 2}]}}
```

Each object is one step, not a complete scenario. Prefer registered fixture drivers for new setup. Legacy `call_method` is restricted to explicit `qa_` fixture methods. `teleport_node` and its alias `fixture_teleport` reset transient physics velocity by default; use `preserve_velocity: true` only when continuing motion is intentional.

Read [Runtime instrumentation](runtime-instrumentation.md) when adding project adapters. Read [Bundle gates](bundle-gates.md) before introducing `godot_args`, multi-viewport execution, or fixed-FPS changes in a warm-strict suite.

## Recordings, seeds, and controllers

`scenario record` produces candidate flows and `review.suggested_assertions`. Review and edit suggestions manually, lint, then rerun. Recording does not approve assertions or baselines; there is no `scenario approve` command.

For generator or difficulty coverage, use `scenario sweep <name> --seeds 1,2,3` or `--seed-start N --seed-count K`, with `--project <project> --json`. Each seed runs cold, the sweep seed overrides the scenario seed, and failures do not abort the sweep. Inspect the aggregate `sweep.json`; use `--require-all-pass` when every seed must pass.

`joy_connect` and `joy_disconnect` simulate connection signals for virtual devices; they do not change native `Input.get_connected_joypads()`. Polling projects may use a QA-only roster seam backed by `QaRuntime.effective_connected_joypads()`. Drive gameplay with `input_joy_state` or `input_joy_button`, and assert assignment, held-input suppression, disconnect, and explicit rebind through game state. Disconnect releases injected buttons/axes; scenario finalization, scene changes, input reset, and live stop clear remaining virtual connections. Synthetic tests do not establish physical controller acceptance.

Optional deeper contract: `docs/scenario-format.md` in a matching godot-qa checkout or distribution. Use its sections on steps, assertions, controller simulation, and templates only as needed.
