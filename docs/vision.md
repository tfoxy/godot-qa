# Product Vision

`godot-qa` should make Godot development safer for AI-assisted workflows.

The immediate problem: an AI can edit Godot files but often cannot verify whether the result works visually or interactively. Complex menus, focus navigation, responsive layouts, and controller-driven UI are especially hard to judge from source code alone.

The solution is a reproducible validation layer that gives the agent concrete feedback:

- What scene is running.
- What nodes exist.
- What Control nodes are visible.
- Where controls are on screen.
- What text each control shows.
- Which control owns focus.
- What happens after input actions.
- What the viewport looks like.
- What errors/warnings/logs occurred.
- Which assertions failed and why.

## Non-goals

- Replace Godot's editor.
- Become a general game automation framework for every engine.
- Guarantee that a UI is beautiful.
- Let AI auto-approve visual changes.
- Require MCP for the core workflow.

One deliberate, bounded extension sits next to that second non-goal: the `drive_policy` scenario step lets a game-registered policy emit real per-device gamepad input each tick so scenarios can validate that mechanics are actually playable in closed loop, not just that a replayed input script still lines up. That is validation-scoped input generation inside the existing scenario/assertion contract — general game automation (botting frameworks, engine-agnostic drivers, gameplay scripting) remains out of scope.

## Success criteria

The current toolkit supports this feedback loop:

```bash
godot-qa scenario run sample_settings_layout --project examples/sample_project --json
# read exact failure: ApplyButton outside viewport at 1280x720
# edit scene/script/theme
godot-qa scenario run sample_settings_layout --project examples/sample_project --json
# pass
```

Live sessions support interactive investigation:

```bash
godot-qa session start --project examples/sample_project --scene res://ui/main_menu.tscn --viewport 1920x1080
godot-qa input click qa:start_button --project examples/sample_project
godot-qa snapshot --project examples/sample_project --name after_start
godot-qa assert visible qa:pause_menu
godot-qa assert focused qa:resume_button
```

Current runtime contracts also cover important non-interactive HUD controls and bounded 3D world/camera visibility. The [CLI reference](cli-spec.md) defines the supported surface and its limits.
