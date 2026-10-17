//! Structural diff of two parsed scene versions into a [`SceneDiff`].
//!
//! Matching is primarily by relative NodePath. A node present only in the old scene is
//! removed; only in the new scene is created/instantiated; present in both is
//! property-diffed. A conservative rename/reparent heuristic collapses a removed+added
//! pair with the same type and immediate-children into a state-preserving
//! [`SceneEditOp::ReparentNode`]. Anything that cannot be faithfully expressed as live
//! edits records a `fell_back` reason so the caller full-scene-reloads instead.

use std::collections::{BTreeMap, HashSet};

use crate::model::{SceneNode, SceneState};
use crate::ops::{SceneDiff, SceneEditOp};
use crate::value::GodotValue;

/// Diff `old` → `new`, producing ordered live-edit ops or fallback reasons.
pub fn diff(old: &SceneState, new: &SceneState) -> SceneDiff {
    let mut result = SceneDiff::default();

    // Parse errors on either side are not safe to diff.
    if old.has_errors() || new.has_errors() {
        result.fell_back.push("scene has parse errors".to_string());
        return result;
    }

    // The root cannot be reparented/recreated live; a change to its identity forces a
    // full reload.
    match (old.root(), new.root()) {
        (Some(old_root), Some(new_root)) => {
            if old_root.name != new_root.name
                || old_root.type_name != new_root.type_name
                || old_root.instance_scene != new_root.instance_scene
                || old_root.is_instance() != new_root.is_instance()
            {
                result
                    .fell_back
                    .push("scene root identity changed".to_string());
                return result;
            }
        }
        _ => {
            result.fell_back.push("scene has no root node".to_string());
            return result;
        }
    }

    let old_by_path = index_by_path(old);
    let new_by_path = index_by_path(new);
    let old_children = child_names_by_path(old);
    let new_children = child_names_by_path(new);

    // Partition non-root paths into removed / added / common.
    let removed_paths: Vec<String> = old_by_path
        .keys()
        .filter(|path| *path != "." && !new_by_path.contains_key(*path))
        .cloned()
        .collect();
    let added_paths: Vec<String> = new_by_path
        .keys()
        .filter(|path| *path != "." && !old_by_path.contains_key(*path))
        .cloned()
        .collect();
    let common_paths: Vec<String> = old_by_path
        .keys()
        .filter(|path| new_by_path.contains_key(*path))
        .cloned()
        .collect();

    // Rename/reparent heuristic: pair a removed node with an added node of the same type
    // and identical immediate-children names, differing only in parent and/or name.
    // Preserves the live node's state instead of remove+create.
    let mut reparents: Vec<(String, String)> = Vec::new();
    let mut matched_added: HashSet<String> = HashSet::new();
    for old_path in &removed_paths {
        let old_node = old_by_path[old_path];
        if old_node.is_instance() {
            continue;
        }
        let old_kids = old_children.get(old_path);
        for new_path in &added_paths {
            if matched_added.contains(new_path) {
                continue;
            }
            let new_node = new_by_path[new_path];
            if new_node.type_name != old_node.type_name || new_node.is_instance() {
                continue;
            }
            let same_kids = old_kids == new_children.get(new_path);
            let moved_or_renamed =
                old_node.name != new_node.name || old_node.parent != new_node.parent;
            if same_kids && moved_or_renamed {
                matched_added.insert(new_path.clone());
                reparents.push((old_path.clone(), new_path.clone()));
                break;
            }
        }
    }
    let matched_removed: HashSet<String> = reparents.iter().map(|(old, _)| old.clone()).collect();

    // 1. Removes (unmatched), deepest first.
    let mut pure_removes: Vec<String> = removed_paths
        .iter()
        .filter(|path| !matched_removed.contains(*path))
        .cloned()
        .collect();
    pure_removes.sort_by_key(|path| std::cmp::Reverse(depth(path)));
    for path in pure_removes {
        result.ops.push(SceneEditOp::RemoveNode { path });
    }

    // 2. Reparents (state-preserving moves/renames), then any property changes on them.
    for (old_path, new_path) in &reparents {
        let old_node = old_by_path[old_path];
        let new_node = new_by_path[new_path];
        let new_parent = new_node.parent_path().unwrap_or_else(|| ".".to_string());
        result.ops.push(SceneEditOp::ReparentNode {
            path: old_path.clone(),
            new_parent,
            new_name: new_node.name.clone(),
            pos: new_node.index.unwrap_or(-1),
        });
        if let Err(reason) = push_changed_properties(&mut result, new_path, old_node, new_node) {
            result.fell_back.push(reason);
            return result;
        }
    }

    // 3. Creates/instantiates (unmatched adds), shallowest first so a parent exists
    //    before its children, then set the new node's authored properties.
    let mut pure_adds: Vec<String> = added_paths
        .iter()
        .filter(|path| !matched_added.contains(*path))
        .cloned()
        .collect();
    pure_adds.sort_by_key(|path| depth(path));
    for path in pure_adds {
        let node = new_by_path[&path];
        let parent = node.parent_path().unwrap_or_else(|| ".".to_string());
        if node.is_instance() {
            match &node.instance_scene {
                Some(scene_path) => result.ops.push(SceneEditOp::InstantiateNode {
                    parent,
                    scene_path: scene_path.clone(),
                    name: node.name.clone(),
                }),
                None => {
                    result
                        .fell_back
                        .push(format!("unresolved instance for node '{}'", node.name));
                    return result;
                }
            }
        } else {
            let Some(type_name) = &node.type_name else {
                result
                    .fell_back
                    .push(format!("new node '{}' has no type", node.name));
                return result;
            };
            result.ops.push(SceneEditOp::CreateNode {
                parent,
                type_name: type_name.clone(),
                name: node.name.clone(),
            });
        }
        for (prop, value) in &node.properties {
            if let Err(reason) = push_single_property(&mut result, &path, prop, value) {
                result.fell_back.push(reason);
                return result;
            }
        }
    }

    // 4. Property changes on nodes present in both versions (including the root, whose
    //    identity was already validated above).
    for path in &common_paths {
        let old_node = old_by_path[path];
        let new_node = new_by_path[path];
        if old_node.type_name != new_node.type_name
            || old_node.is_instance() != new_node.is_instance()
            || old_node.instance_scene != new_node.instance_scene
        {
            result
                .fell_back
                .push(format!("node '{path}' changed type or instance in place"));
            return result;
        }
        if let Err(reason) = push_changed_properties(&mut result, path, old_node, new_node) {
            result.fell_back.push(reason);
            return result;
        }
    }

    result
}

/// Emit `SetProp`/`SetPropRes` for every property that changed (added or modified).
/// Removed properties cannot be "unset" live and are left as-is (documented limitation).
fn push_changed_properties(
    result: &mut SceneDiff,
    path: &str,
    old_node: &SceneNode,
    new_node: &SceneNode,
) -> Result<(), String> {
    for (prop, value) in &new_node.properties {
        if old_node.property(prop) != Some(value) {
            push_single_property(result, path, prop, value)?;
        }
    }
    Ok(())
}

/// Emit one property op, routing resource-valued props through `SetPropRes` and
/// rejecting unsupported values.
fn push_single_property(
    result: &mut SceneDiff,
    path: &str,
    prop: &str,
    value: &GodotValue,
) -> Result<(), String> {
    match value {
        GodotValue::Unsupported(reason) => Err(format!(
            "property '{prop}' on '{path}' has an unsupported value ({reason})"
        )),
        GodotValue::ExtResource(res_path) => {
            result.ops.push(SceneEditOp::SetPropRes {
                path: path.to_string(),
                prop: prop.to_string(),
                res_path: res_path.clone(),
            });
            Ok(())
        }
        _ => {
            result.ops.push(SceneEditOp::SetProp {
                path: path.to_string(),
                prop: prop.to_string(),
                value: value.clone(),
            });
            Ok(())
        }
    }
}

fn index_by_path(scene: &SceneState) -> BTreeMap<String, &SceneNode> {
    let mut map = BTreeMap::new();
    for node in &scene.nodes {
        map.insert(node.relative_path(), node);
    }
    map
}

/// Map each node's relative path to the sorted list of its immediate children's names.
fn child_names_by_path(scene: &SceneState) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for node in &scene.nodes {
        if let Some(parent) = node.parent_path() {
            map.entry(parent).or_default().push(node.name.clone());
        }
    }
    for names in map.values_mut() {
        names.sort();
    }
    map
}

fn depth(path: &str) -> usize {
    if path == "." {
        0
    } else {
        path.split('/').count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(
        name: &str,
        type_name: Option<&str>,
        parent: Option<&str>,
        props: &[(&str, GodotValue)],
    ) -> SceneNode {
        SceneNode {
            name: name.to_string(),
            type_name: type_name.map(str::to_string),
            parent: parent.map(str::to_string),
            instance_scene: None,
            instance_unresolved: false,
            index: None,
            groups: Vec::new(),
            properties: props
                .iter()
                .map(|(key, value)| (key.to_string(), value.clone()))
                .collect(),
        }
    }

    fn scene(nodes: Vec<SceneNode>) -> SceneState {
        SceneState {
            nodes,
            ..SceneState::default()
        }
    }

    fn root() -> SceneNode {
        node("Main", Some("Node2D"), None, &[])
    }

    #[test]
    fn identical_scenes_produce_no_ops() {
        let a = scene(vec![
            root(),
            node("Player", Some("Sprite2D"), Some("."), &[]),
        ]);
        let out = diff(&a, &a);
        assert!(out.ops.is_empty());
        assert!(!out.requires_full_reload());
    }

    #[test]
    fn added_node_creates_and_sets_props() {
        let old = scene(vec![root()]);
        let new = scene(vec![
            root(),
            node(
                "HUD",
                Some("Control"),
                Some("."),
                &[("visible", GodotValue::Bool(true))],
            ),
        ]);
        let out = diff(&old, &new);
        assert!(!out.requires_full_reload());
        assert_eq!(
            out.ops,
            vec![
                SceneEditOp::CreateNode {
                    parent: ".".to_string(),
                    type_name: "Control".to_string(),
                    name: "HUD".to_string(),
                },
                SceneEditOp::SetProp {
                    path: "HUD".to_string(),
                    prop: "visible".to_string(),
                    value: GodotValue::Bool(true),
                },
            ]
        );
    }

    #[test]
    fn creates_are_ordered_parent_before_child() {
        let old = scene(vec![root()]);
        let new = scene(vec![
            root(),
            node("UI", Some("Control"), Some("."), &[]),
            node("HUD", Some("Label"), Some("UI"), &[]),
        ]);
        let out = diff(&old, &new);
        assert_eq!(
            out.ops,
            vec![
                SceneEditOp::CreateNode {
                    parent: ".".to_string(),
                    type_name: "Control".to_string(),
                    name: "UI".to_string(),
                },
                SceneEditOp::CreateNode {
                    parent: "UI".to_string(),
                    type_name: "Label".to_string(),
                    name: "HUD".to_string(),
                },
            ]
        );
    }

    #[test]
    fn removed_node_emits_remove() {
        let old = scene(vec![
            root(),
            node("Player", Some("Sprite2D"), Some("."), &[]),
        ]);
        let new = scene(vec![root()]);
        let out = diff(&old, &new);
        assert_eq!(
            out.ops,
            vec![SceneEditOp::RemoveNode {
                path: "Player".to_string()
            }]
        );
    }

    #[test]
    fn changed_property_emits_set_prop() {
        let old = scene(vec![
            root(),
            node(
                "Player",
                Some("Sprite2D"),
                Some("."),
                &[("frame", GodotValue::Int(1))],
            ),
        ]);
        let new = scene(vec![
            root(),
            node(
                "Player",
                Some("Sprite2D"),
                Some("."),
                &[("frame", GodotValue::Int(2))],
            ),
        ]);
        let out = diff(&old, &new);
        assert_eq!(
            out.ops,
            vec![SceneEditOp::SetProp {
                path: "Player".to_string(),
                prop: "frame".to_string(),
                value: GodotValue::Int(2),
            }]
        );
    }

    #[test]
    fn ext_resource_property_routes_to_set_prop_res() {
        let old = scene(vec![
            root(),
            node("Player", Some("Sprite2D"), Some("."), &[]),
        ]);
        let new = scene(vec![
            root(),
            node(
                "Player",
                Some("Sprite2D"),
                Some("."),
                &[(
                    "texture",
                    GodotValue::ExtResource("res://icon.png".to_string()),
                )],
            ),
        ]);
        let out = diff(&old, &new);
        assert_eq!(
            out.ops,
            vec![SceneEditOp::SetPropRes {
                path: "Player".to_string(),
                prop: "texture".to_string(),
                res_path: "res://icon.png".to_string(),
            }]
        );
    }

    #[test]
    fn reparent_preserves_node_as_move() {
        // Same node (name+type+children) moves from root to under "UI".
        let old = scene(vec![
            root(),
            node("UI", Some("Control"), Some("."), &[]),
            node("Score", Some("Label"), Some("."), &[]),
        ]);
        let new = scene(vec![
            root(),
            node("UI", Some("Control"), Some("."), &[]),
            node("Score", Some("Label"), Some("UI"), &[]),
        ]);
        let out = diff(&old, &new);
        assert!(!out.requires_full_reload());
        assert_eq!(
            out.ops,
            vec![SceneEditOp::ReparentNode {
                path: "Score".to_string(),
                new_parent: "UI".to_string(),
                new_name: "Score".to_string(),
                pos: -1,
            }]
        );
    }

    #[test]
    fn root_type_change_falls_back() {
        let old = scene(vec![node("Main", Some("Node2D"), None, &[])]);
        let new = scene(vec![node("Main", Some("Node3D"), None, &[])]);
        let out = diff(&old, &new);
        assert!(out.requires_full_reload());
    }

    #[test]
    fn in_place_type_change_falls_back() {
        let old = scene(vec![root(), node("X", Some("Sprite2D"), Some("."), &[])]);
        let new = scene(vec![
            root(),
            node("X", Some("AnimatedSprite2D"), Some("."), &[]),
        ]);
        let out = diff(&old, &new);
        assert!(out.requires_full_reload());
    }

    #[test]
    fn unsupported_value_falls_back() {
        let old = scene(vec![root(), node("X", Some("Sprite2D"), Some("."), &[])]);
        let new = scene(vec![
            root(),
            node(
                "X",
                Some("Sprite2D"),
                Some("."),
                &[("mesh", GodotValue::Unsupported("SubResource".to_string()))],
            ),
        ]);
        let out = diff(&old, &new);
        assert!(out.requires_full_reload());
    }
}
