# QA IDs and Target Locators

Runtime UI automation needs stable targets. Node paths are useful but fragile. Prefer `qa_id`.

## Recommended Godot pattern

Use `QaRuntime.register_qa_id()` when the addon helper is available. It records the semantic `qa_id` metadata and joins the canonical fast-path `qa.id.<id>` group in one place:

```gdscript
const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")

func _ready() -> void:
    QaRuntime.register_qa_id($StartButton, "start_button")
```

The equivalent manual pattern is metadata plus the canonical group:

```gdscript
func _ready() -> void:
    $StartButton.set_meta("qa_id", "start_button")
    $StartButton.add_to_group("qa.id.start_button")
```

Older scenes may also keep compatibility groups such as `qa.start_button`, but new code should prefer `qa.id.start_button`.

This pattern is intended for stable controls, controllers, important containers, player roots, and fixture anchors that scenarios need to address directly. Keep the ID tied to the role the node plays, not to its current tree position.

Target from CLI/scenario:

```text
qa:start_button
```

## Locator priority

1. `qa:<id>` canonical group convention `qa.id.<id>` as the efficient fast path.
2. `qa:<id>` compatibility group `qa.<id>` for older scenes.
3. Recursive fallback scan for `qa_id` metadata so metadata-only projects remain supported.
4. Recursive fallback scan for `qa_aliases` metadata entries containing raw ids or prefixed `qa:` locators.
5. `path:<absolute-node-path>`.
6. `name:<node-name>` only for diagnostics or quick exploration.

Group lookup is the preferred runtime fast path because Godot can return candidates without scanning every node. Metadata remains the semantic compatibility contract: `qa_id` identifies the node's durable role, and `qa_aliases` can preserve old locators during refactors. When a node is found through metadata, canonical groups, compatibility groups, and aliases, the resolver deduplicates the same node and counts it once.

## Durable anchors vs dynamic state

Not every gameplay fact should become a runtime locator. Use a small set of durable anchors for automation entry points, then ask those anchors for structured state or probes when the data is dynamic.

| Need | Recommended pattern |
| --- | --- |
| Stable unique target such as a button, controller, player root, or fixture anchor | Use `QaRuntime.register_qa_id(node, "<id>")`, or set `qa_id` metadata plus `qa.id.<id>`, and target it as `qa:<id>`. |
| Compatibility with older scenes or bulk categorization | Keep metadata-only IDs, `qa_aliases`, or compatibility groups such as `qa.<id>` during migration; use normal project groups for broad categories. |
| Many generated objects such as cells, projectiles, pickups, debris, or temporary effects | Avoid per-object `qa_id` metadata and repeated metadata scans; expose aggregate state from a durable anchor instead. |
| Aggregate gameplay data such as scores, entities, cooldowns, objectives, or interaction summaries | Prefer `QaRuntime.register_state_provider("<provider_id>", owner, "<method>")` and assert with `source: provider`. Legacy `get_qa_*_state()` and `get_*_qa_state()` methods on durable anchors remain supported. |
| Legacy aggregate gameplay data | Implement `get_qa_*_state()` on a durable anchor, or use a named `get_*_qa_state()` aggregate when that better describes the system, and return JSON-compatible dictionaries and arrays. |
| Spatial, collision, walkability, or surface facts | Prefer `QaRuntime.register_probe_provider("<provider_id>", owner, "<method>", {"kind": "surface"})` and call it with `provider_id`. Legacy `qa_*_probe()` provider methods remain supported. |
| Legacy spatial, collision, walkability, or surface facts | Implement a named `qa_*_probe()` provider method that classifies the requested point, shape, or query. |
| Deterministic setup that mutates gameplay | Prefer `QaRuntime.register_fixture_driver("<driver_id>", owner, "<method>")` and scenario `fixture_call`; keep this separate from read-only state/probe providers. |
| Intentionally stable engine-owned node or low-level diagnostic target | Use `path:` when the path itself is the thing being diagnosed. |
| Quick exploration while authoring scenarios | Use `name:` only to inspect likely targets, then replace it with `qa:` or a state/probe assertion. |

State method assertions accept either registered state providers or legacy `get_qa_*_state()` or `get_*_qa_state()` aggregate methods. For new projects, use runtime inspection/reflection first for engine-visible facts, registered read-only providers second for semantic aggregate state, and fixture drivers third for deterministic setup.

QA ID lookup is appropriate for stable anchors because the match set should be small and unique. High-cardinality dynamic objects make metadata scans slower, especially when the resolver must use fallback scanning, and create brittle scenarios when runtime spawning, pooling, or despawning changes the scene tree. Prefer logical IDs inside returned state dictionaries for generated gameplay objects; those logical IDs describe domain objects and are not runtime node locators.

## State and probe examples

A match controller can be a single durable anchor while returning dynamic gameplay state:

```gdscript
func _ready() -> void:
    QaRuntime.register_qa_id(self, "match_controller")
    QaRuntime.register_state_provider("match_state", self, "_qa_match_state")

func _qa_match_state() -> Dictionary:
    return {
        "phase": phase,
        "players": players.map(func(player):
            return {
                "id": player.logical_id,
                "health": player.health,
                "position": [player.global_position.x, player.global_position.y]
            }
        ),
        "projectiles": projectiles.map(func(projectile):
            return {
                "id": projectile.logical_id,
                "owner_id": projectile.owner_id,
                "position": [projectile.global_position.x, projectile.global_position.y]
            }
        )
    }
```

Scenario assertions can target `qa:match_controller` and call `get_qa_match_state()` or a named aggregate such as `get_raft_motion_qa_state()`, or use `source: provider` and `provider: match_state` instead of assigning one `qa_id` to every generated player clone, projectile, or pickup. The `id` fields in the returned dictionaries are logical gameplay identifiers, not `qa:` locators.

```yaml
assert:
  - state_matches:
      source: provider
      provider: match_state
      path: phase
      operator: equals
      expected: gameplay
```

A surface provider can classify points without exposing every tile or collision shape as a locator:

```gdscript
func _ready() -> void:
    QaRuntime.register_probe_provider("surface_main", self, "_surface_probe", {"kind": "surface"})

func _surface_probe(request: Dictionary) -> Dictionary:
    var points := []
    for point in request.get("points", []):
        var x := float(point.get("x", 0.0))
        var y := float(point.get("y", 0.0))
        var tile := local_to_map(Vector2(x, y))
        points.append({
            "x": x,
            "y": y,
            "surface": get_surface_name(tile),
            "walkable": is_tile_walkable(tile)
        })
    return {
        "points": points
    }
```

Scenario assertions can target `qa:surface_provider` and call `qa_surface_probe()` for spatial, collision, walkability, or surface facts. New scenarios can pass `provider_id: surface_main` instead. Provider IDs are not node locators and cannot be used for click, visibility, rect, or focus commands.

## Ambiguity handling

If a locator matches multiple nodes, return a failure:

```json
{
  "type": "ambiguous_target",
  "target": "qa:start_button",
  "expected": "unique runtime node",
  "actual": [
    "path:/root/MenuA/StartButton",
    "path:/root/MenuB/StartButton"
  ]
}
```

Do not silently pick the first match.

Ambiguity lists distinct matching nodes once, even when the same node matched through both `qa.id.<id>` and `qa_id` or `qa_aliases`. The `actual` field contains deduplicated `path:/root/...` locators for the distinct matches.

If a fast-path group contains candidates that cannot resolve safely, the resolver rejects them and either falls back to metadata/alias scanning or returns `target_not_found` with diagnostics. Missing-target payloads may include `candidate_status` and `rejected_candidates` entries for stale, freed, outside-tree, or mismatched group members so projects can repair old registrations.
