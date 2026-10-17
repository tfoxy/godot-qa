# Layout Audit Rules

Layout audits are the high-value feature for AI-generated UI.

The goal is not to judge aesthetics. The goal is to catch objective layout failures and produce repairable diagnostics.

## Rule categories

### Viewport containment

Fail when important visible controls are outside the viewport or safe area.

```text
outside_viewport
outside_safe_area
panel_larger_than_viewport
```

### Overlap

Fail when interactive controls overlap in a way that likely makes them unusable.

```text
interactive_overlap
important_overlap
modal_background_above_dialog
```

Non-interactive decorative overlaps should be ignored or warning-only.

### Minimum target size

Warn or fail when clickable/focusable controls are too small.

```text
click_target_too_small
focus_target_too_small
```

Default suggested minimum: configurable; start with 32x32 for strict games and 44x44 for accessibility-oriented menus.

### Text clipping

Detect likely text clipping through Control properties and bounding boxes.

```text
text_likely_clipped
label_zero_size
button_text_empty
```

Precise text rendering measurement can come later.

### Focus/navigation

Godot UI/gamepad navigation often fails when focus is not explicit.

```text
missing_initial_focus
focus_unreachable
focus_cycle_trap
missing_cancel_path
ambiguous_focus_neighbor
```

The audit should identify:

- current focus owner;
- all focusable controls;
- expected initial focus when provided;
- navigation path results for common actions.

### Modal/dialog contracts

```text
modal_without_focused_button
modal_without_cancel
modal_button_outside_dialog
```

### Scroll containers

```text
scroll_content_clipped
focused_control_outside_visible_scroll_area
scrollbar_missing_when_needed
```

## Severity levels

```text
error    Fails scenario/layout audit.
warning  Reported but does not fail unless --strict.
info     Diagnostic only.
```

For S20, the implemented policy still starts narrower than the full rule catalog above. The shipped commands focus on objective, runtime-derived checks over `ui.json` and `focus.json`: viewport containment, interactive overlap, and missing initial focus. The same CLI rules now apply to both saved snapshot artifacts and live-session snapshots captured through `layout audit-current --live`. More advanced text, scroll, modal, and navigation heuristics remain later work.

## `layout_audit` in `scenario run`

A `layout_audit` assertion audits a saved snapshot, so the run must capture one (add a `snapshot` step). Routing depends on the `viewports` count:

- **Zero or one `viewports` entry** — the scenario runs the normal single-run path. The audit is a CLI post-process folded over the run's final snapshot, and its geometry failures attach inline to the ordinary single-run result envelope (no `runs[]` matrix). A single `viewports` entry sets the window resolution. Because it is a post-process, `layout_audit` and `screenshot_match` can both apply to the same single run.
- **Two or more `viewports` entries** — the scenario fans out into the viewport matrix (`data.viewports` + `data.runs[]`), auditing each cell's last snapshot.

Either path fails with `layout_audit_missing_snapshot` when the run recorded no snapshot to audit — the historical silent pass (an empty failure set with no evidence) is gone. A viewport matrix that resolves to zero runs fails with `viewport_matrix_empty` rather than passing vacuously.

## Suggested command output

```json
{
  "ok": false,
  "failures": [
    {
      "type": "interactive_overlap",
      "severity": "error",
      "viewport": [1280, 720],
      "a": "qa:music_slider",
      "b": "qa:sfx_slider",
      "rect_a": [400, 312, 500, 48],
      "rect_b": [400, 340, 500, 48]
    }
  ]
}
```

## Recommended first implementation

Start with deterministic geometry rules:

1. Dump visible `Control` nodes.
2. Filter to interactive/focusable/important controls.
3. Check containment inside viewport.
4. Check interactive overlaps.
5. Check initial focus.
6. Check basic focus path if expected in scenario.

Add text measurement and scroll details later.
