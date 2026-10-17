//! The structural edit operations the differ emits. These map 1:1 onto Godot's
//! `scene:live_*` debugger messages (encoded by `godot-qa-debug`). All paths are
//! NodePaths relative to the scene root; the game re-applies each op to every live
//! instance whose `scene_file_path` matches the edited scene.

use serde::{Deserialize, Serialize};

use crate::value::GodotValue;

/// A single structural edit to apply to the running scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SceneEditOp {
    /// `live_create_node[parent, type, name]` — add a fresh built-in node.
    CreateNode {
        parent: String,
        type_name: String,
        name: String,
    },
    /// `live_instantiate_node[parent, scene_path, name]` — add an instanced sub-scene.
    InstantiateNode {
        parent: String,
        scene_path: String,
        name: String,
    },
    /// `live_remove_node[path]` — delete a node (and its subtree).
    RemoveNode { path: String },
    /// `live_reparent_node[path, new_parent, new_name, pos]` — move/rename in place,
    /// preserving the live node's state.
    ReparentNode {
        path: String,
        new_parent: String,
        new_name: String,
        pos: i64,
    },
    /// `live_node_prop[id, prop, value]` — set a property to an encodable Variant value.
    SetProp {
        path: String,
        prop: String,
        value: GodotValue,
    },
    /// `live_node_prop_res[id, prop, res_path]` — set a property to a resource by path.
    SetPropRes {
        path: String,
        prop: String,
        res_path: String,
    },
}

/// The result of diffing two scene versions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneDiff {
    /// Ordered ops to apply. Empty when the scenes are structurally identical.
    #[serde(default)]
    pub ops: Vec<SceneEditOp>,
    /// Non-empty when the delta cannot be faithfully expressed as live edits; each
    /// string is a human-readable reason. The caller should full-scene-reload instead.
    #[serde(default)]
    pub fell_back: Vec<String>,
}

impl SceneDiff {
    /// True when the diff must be resolved by a full-scene reload rather than live edits.
    pub fn requires_full_reload(&self) -> bool {
        !self.fell_back.is_empty()
    }
}
