//! Structural diff of two parsed scene versions into a [`SceneDiff`].
//!
//! Matching is primarily by relative NodePath. A node present only in the old scene is
//! removed; only in the new scene is created/instantiated; present in both is
//! property-diffed. A conservative rename/reparent heuristic collapses a removed+added
//! pair with the same type and immediate-children into a state-preserving
//! [`SceneEditOp::ReparentNode`]. Anything that cannot be faithfully expressed as live
//! edits records a `fell_back` reason so the caller full-scene-reloads instead.

use std::collections::{BTreeMap, HashSet, VecDeque};

use crate::model::{SceneNode, SceneState};
use crate::ops::{SceneDiff, SceneEditOp};
use crate::value::GodotValue;

#[cfg(test)]
thread_local! {
    static RENAME_CANDIDATE_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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

    if let Some(id) = changed_referenced_sub_resource(old, new) {
        result.fell_back.push(format!(
            "referenced or potentially referenced SubResource '{id}' changed"
        ));
        return result;
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
    // Index by type and child names, retaining path order within each bucket so
    // ties match exactly as in the original ordered scan.
    let mut candidates = BTreeMap::new();
    for new_path in &added_paths {
        let new_node = new_by_path[new_path];
        if !new_node.is_instance() {
            candidates
                .entry((new_node.type_name.as_deref(), new_children.get(new_path)))
                .or_insert_with(VecDeque::new)
                .push_back(new_path);
        }
    }
    let mut reparents: Vec<(String, String)> = Vec::new();
    let mut matched_added: HashSet<String> = HashSet::new();
    for old_path in &removed_paths {
        let old_node = old_by_path[old_path];
        if old_node.is_instance() {
            continue;
        }
        if let Some(new_path) = candidates
            .get_mut(&(old_node.type_name.as_deref(), old_children.get(old_path)))
            .and_then(VecDeque::pop_front)
        {
            #[cfg(test)]
            RENAME_CANDIDATE_CHECKS.with(|checks| checks.set(checks.get() + 1));
            // Removed and added paths are disjoint, so the parent or name differs.
            matched_added.insert(new_path.clone());
            reparents.push((old_path.clone(), new_path.clone()));
        }
    }
    let matched_removed: HashSet<String> = reparents.iter().map(|(old, _)| old.clone()).collect();

    // Moving a subtree changes descendant paths, and removes precede moves while
    // creates follow them. Reject dependent moves before emitting any operations.
    for (old_path, new_path) in &reparents {
        let old_parent = old_by_path[old_path].parent_path().unwrap();
        let new_parent = new_by_path[new_path].parent_path().unwrap();
        let dependency = if old_children.contains_key(old_path) {
            Some("node has retained children".to_string())
        } else if !new_by_path.contains_key(&old_parent) {
            Some(format!("source parent '{old_parent}' is removed or moved"))
        } else if !old_by_path.contains_key(&new_parent) {
            Some(format!(
                "destination parent '{new_parent}' is added or moved"
            ))
        } else {
            None
        };
        if let Some(reason) = dependency {
            result.fell_back.push(format!(
                "cannot move '{old_path}' to '{new_path}' live: {reason}"
            ));
            return result;
        }
    }

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

/// Follow inline-resource references, including nested collections and resources.
/// Unreferenced resource edits do not affect the live scene.
fn changed_referenced_sub_resource<'a>(
    old: &'a SceneState,
    new: &'a SceneState,
) -> Option<&'a str> {
    let old_resources: BTreeMap<_, _> = old
        .sub_resources
        .iter()
        .map(|resource| (resource.id.as_str(), resource))
        .collect();
    let new_resources: BTreeMap<_, _> = new
        .sub_resources
        .iter()
        .map(|resource| (resource.id.as_str(), resource))
        .collect();
    let mut pending = Vec::new();
    let mut opaque_references = false;
    for node in old.nodes.iter().chain(&new.nodes) {
        for (_, value) in &node.properties {
            collect_sub_resource_ids(value, &mut pending, &mut opaque_references);
        }
    }
    let mut visited = HashSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let previous = old_resources.get(id);
        let current = new_resources.get(id);
        if previous != current {
            return Some(id);
        }
        if let Some(resource) = current {
            for (_, value) in &resource.properties {
                collect_sub_resource_ids(value, &mut pending, &mut opaque_references);
            }
        }
    }
    // Unsupported constructors discard their arguments, so reachable opaque values
    // may hide references to any inline resource. Conservatively reload on any edit.
    if opaque_references {
        return old_resources
            .keys()
            .chain(new_resources.keys())
            .copied()
            .find(|id| old_resources.get(id) != new_resources.get(id));
    }
    None
}

fn collect_sub_resource_ids<'a>(
    value: &'a GodotValue,
    ids: &mut Vec<&'a str>,
    opaque_references: &mut bool,
) {
    match value {
        GodotValue::Unsupported(reason) => {
            if let Some(id) = reason.strip_prefix("SubResource:") {
                ids.push(id);
            } else if !reason.starts_with("ExtResource:") {
                *opaque_references = true;
            }
        }
        GodotValue::Array(values) => {
            for value in values {
                collect_sub_resource_ids(value, ids, opaque_references);
            }
        }
        GodotValue::Dictionary(entries) => {
            for (key, value) in entries {
                collect_sub_resource_ids(key, ids, opaque_references);
                collect_sub_resource_ids(value, ids, opaque_references);
            }
        }
        _ => {}
    }
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
    fn large_disjoint_types_do_not_scan_all_rename_candidates() {
        const COUNT: usize = 10_000;
        let mut old = scene(vec![root()]);
        let mut new = scene(vec![root()]);
        for index in 0..COUNT {
            old.nodes.push(node(
                &format!("Old{index:05}"),
                Some("Node2D"),
                Some("."),
                &[],
            ));
            new.nodes.push(node(
                &format!("New{index:05}"),
                Some("Node3D"),
                Some("."),
                &[],
            ));
        }
        assert_disjoint_rename_work(&old, &new, COUNT);
    }

    #[test]
    fn large_disjoint_child_signatures_do_not_scan_all_rename_candidates() {
        const COUNT: usize = 1_000;
        let mut old = scene(vec![root()]);
        let mut new = scene(vec![root()]);
        for index in 0..COUNT {
            let old_parent = format!("Old{index:05}");
            let new_parent = format!("New{index:05}");
            old.nodes
                .push(node(&old_parent, Some("Node"), Some("."), &[]));
            old.nodes
                .push(node("OldChild", Some("Node2D"), Some(&old_parent), &[]));
            new.nodes
                .push(node(&new_parent, Some("Node"), Some("."), &[]));
            new.nodes
                .push(node("NewChild", Some("Node3D"), Some(&new_parent), &[]));
        }
        assert_disjoint_rename_work(&old, &new, COUNT * 2);
    }

    fn assert_disjoint_rename_work(old: &SceneState, new: &SceneState, count: usize) {
        RENAME_CANDIDATE_CHECKS.with(|checks| checks.set(0));
        let out = diff(old, new);
        assert!(!out.requires_full_reload(), "{:?}", out.fell_back);
        assert_eq!(out.ops.len(), count * 2);
        assert_eq!(
            out.ops
                .iter()
                .filter(|op| matches!(op, SceneEditOp::RemoveNode { .. }))
                .count(),
            count
        );
        assert_eq!(
            out.ops
                .iter()
                .filter(|op| matches!(op, SceneEditOp::CreateNode { .. }))
                .count(),
            count
        );
        let checks = RENAME_CANDIDATE_CHECKS.with(|checks| checks.get());
        assert!(
            checks <= count,
            "expected at most {count} candidate checks, got {checks}"
        );
    }

    #[test]
    fn ambiguous_rename_candidates_keep_path_order() {
        let old = scene(vec![
            root(),
            node("Y", Some("Label"), Some("."), &[]),
            node("X", Some("Label"), Some("."), &[]),
        ]);
        let new = scene(vec![
            root(),
            node("B", Some("Label"), Some("."), &[]),
            node("A", Some("Label"), Some("."), &[]),
        ]);
        let out = diff(&old, &new);
        assert!(!out.requires_full_reload());
        assert_eq!(
            out.ops,
            vec![
                SceneEditOp::ReparentNode {
                    path: "X".to_string(),
                    new_parent: ".".to_string(),
                    new_name: "A".to_string(),
                    pos: -1,
                },
                SceneEditOp::ReparentNode {
                    path: "Y".to_string(),
                    new_parent: ".".to_string(),
                    new_name: "B".to_string(),
                    pos: -1,
                },
            ]
        );
    }

    #[test]
    fn rename_parent_with_retained_child_falls_back() {
        let old = scene(vec![
            root(),
            node("A", Some("Node"), Some("."), &[]),
            node("Child", Some("Sprite2D"), Some("A"), &[]),
        ]);
        let new = scene(vec![
            root(),
            node("B", Some("Node"), Some("."), &[]),
            node("Child", Some("Sprite2D"), Some("B"), &[]),
        ]);
        let out = diff(&old, &new);
        assert!(out.requires_full_reload(), "{out:?}");
        assert!(out.ops.is_empty(), "{out:?}");
    }

    #[test]
    fn move_into_new_parent_falls_back() {
        let old = scene(vec![
            root(),
            node("Child", Some("Sprite2D"), Some("."), &[]),
        ]);
        let new = scene(vec![
            root(),
            node("Parent", Some("Node"), Some("."), &[]),
            node("Child", Some("Sprite2D"), Some("Parent"), &[]),
        ]);
        let out = diff(&old, &new);
        assert!(out.requires_full_reload(), "{out:?}");
        assert!(out.ops.is_empty(), "{out:?}");
    }

    #[test]
    fn move_out_of_removed_parent_falls_back() {
        let old = scene(vec![
            root(),
            node("Parent", Some("Node"), Some("."), &[]),
            node("Child", Some("Sprite2D"), Some("Parent"), &[]),
        ]);
        let new = scene(vec![
            root(),
            node("Child", Some("Sprite2D"), Some("."), &[]),
        ]);
        let out = diff(&old, &new);
        assert!(out.requires_full_reload(), "{out:?}");
        assert!(out.ops.is_empty(), "{out:?}");
    }

    fn material_scene(reference: &str, color: &str) -> SceneState {
        crate::parse_scene(&format!(
            "[sub_resource type=\"StandardMaterial3D\" id=\"Material\"]\n\
             albedo_color = {color}\n\
             [sub_resource type=\"Resource\" id=\"Wrapper\"]\n\
             material = SubResource(\"Material\")\n\
             [node name=\"Main\" type=\"Node\"]\n\
             material = {reference}\n"
        ))
    }

    #[test]
    fn referenced_sub_resource_edit_falls_back() {
        for reference in [
            "SubResource(\"Material\")",
            "[SubResource(\"Material\")]",
            "{\"material\": SubResource(\"Material\")}",
            "SubResource(\"Wrapper\")",
        ] {
            let old = material_scene(reference, "Color(1, 0, 0, 1)");
            let new = material_scene(reference, "Color(0, 1, 0, 1)");
            assert!(!old.has_errors() && !new.has_errors());
            let out = diff(&old, &new);
            assert!(out.requires_full_reload(), "{reference}: {out:?}");
            assert!(out.ops.is_empty(), "{reference}: {out:?}");
            assert!(out.fell_back[0].contains("Material"));
        }
    }

    #[test]
    fn opaque_constructor_sub_resource_edit_falls_back() {
        for reference in [
            "Array[Material]([SubResource(\"Material\")])",
            "Array[Resource]([SubResource(\"Material\")])",
            "SomeObject(SubResource(\"Material\"))",
            "{\"materials\": Array[Material]([SubResource(\"Material\")])}",
        ] {
            let old = material_scene(reference, "Color(1, 0, 0, 1)");
            let new = material_scene(reference, "Color(0, 1, 0, 1)");
            assert!(old.diagnostics.is_empty(), "{:?}", old.diagnostics);
            assert!(new.diagnostics.is_empty(), "{:?}", new.diagnostics);
            let out = diff(&old, &new);
            assert!(out.requires_full_reload(), "{reference}: {out:?}");
            assert!(out.ops.is_empty(), "{reference}: {out:?}");
            assert!(out.fell_back[0].contains("Material"));
            assert!(!diff(&old, &old).requires_full_reload());
        }
    }

    #[test]
    fn reachable_opaque_sub_resource_edit_falls_back() {
        let reference = "SubResource(\"Wrapper\")";
        let old = material_scene(reference, "Color(1, 0, 0, 1)");
        let new = material_scene(reference, "Color(0, 1, 0, 1)");
        let wrap = |mut scene: SceneState| {
            scene.sub_resources[1].properties[0].1 =
                GodotValue::Unsupported("Array[Material]".to_string());
            scene
        };
        let out = diff(&wrap(old), &wrap(new));
        assert!(out.requires_full_reload(), "{out:?}");
        assert!(out.ops.is_empty(), "{out:?}");
    }

    #[test]
    fn unreferenced_sub_resource_edit_remains_no_op() {
        let old = material_scene("null", "Color(1, 0, 0, 1)");
        let new = material_scene("null", "Color(0, 1, 0, 1)");
        let out = diff(&old, &new);
        assert!(!out.requires_full_reload(), "{out:?}");
        assert!(out.ops.is_empty());
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
