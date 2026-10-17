# Patch Format

Live edits are temporary by default. A patch is a reviewable description of scene-backed changes that may be applied back to source through Godot APIs.

## Current scope

- Format id: `godot-qa.patch.v1`
- Exported operations: `set_property` only
- Patch targets: scene nodes resolved by the same `qa:`, `path:`, and `name:` locators used at runtime
- Supported value types:
  - bare JSON scalars for `String`, `bool`, `int`, `float`, and `null`
  - typed envelopes for Godot-specific values such as `Vector2`, `Vector2i`, and `Color`
- Live-only method calls such as `grab_focus` are tracked in `edit-live diff` but skipped during `edit-live export-patch`

## Example

```json
{
  "format": "godot-qa.patch.v1",
  "scene": "res://ui/settings_menu.tscn",
  "created_at": "2026-04-21T12:34:56Z",
  "created_from": {
    "session_id": "20260421T123456Z",
    "session_meta": "/abs/path/.qa/artifacts/live/20260421T123456Z/meta.json"
  },
  "operations": [
    {
      "op": "set_property",
      "target": "qa:settings_panel",
      "property": "custom_minimum_size",
      "value": {
        "type": "Vector2",
        "value": [900.0, 600.0]
      }
    },
    {
      "op": "set_property",
      "target": "qa:settings_title",
      "property": "text",
      "value": "Settings"
    }
  ]
}
```

## Apply behavior

- Patch application is explicit through `godot-qa apply-patch <patch.json>`.
- The CLI prepares a dedicated artifact root under `.qa/artifacts/patch-apply/<patch-stem>/<run-id>/`.
- Exported patches include the export timestamp plus the live session id/meta artifact they came from.
- Godot validates every operation before saving the scene file.
- If any operation fails, the scene is not saved.
- Scene saves go through `PackedScene.pack()` plus `ResourceSaver.save()`; source files are not edited as raw text.

## Rules

- Patch targets must resolve uniquely.
- Failed operations must keep actionable `type`, `message`, `expected`, `actual`, `target`, and artifact paths when available.
- Patches should stay small and reviewable.
- Script-source edits remain out of scope in S10.
