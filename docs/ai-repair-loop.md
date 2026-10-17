# AI Repair Loop

The core workflow is an evidence-driven loop:

```text
run scenario -> inspect structured failure -> inspect artifacts -> make smallest source change -> rerun scenario
```

## Example

```bash
godot-qa scenario run settings_menu_layout --json
```

Failure:

```json
{
  "type": "outside_viewport",
  "target": "qa:apply_button",
  "viewport": [1280, 720],
  "rect": [1090, 690, 240, 64]
}
```

Agent response:

1. Read `ui.json` for `qa:apply_button` and parent containers.
2. Read `screenshot.png` for visual context.
3. Change the source scene/theme/container layout.
4. Rerun the scenario.
5. Claim success only when the assertion passes.

## Preferred diagnostic order

1. Runtime errors and logs.
2. Scene tree state.
3. UI Control geometry/state.
4. Focus/navigation state.
5. Screenshot comparison.

Screenshots are helpful, but the repair target should usually come from JSON geometry and state.
