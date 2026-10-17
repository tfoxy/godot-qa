//! The parsed `.tscn` scene model. Deliberately close to the reference parser's
//! `GodotSceneState` so the port stays faithful, but geared to what the structural
//! differ needs: an ordered node list with relative-NodePath reconstruction.

use serde::{Deserialize, Serialize};

use crate::value::GodotValue;

/// Severity of a non-fatal parse diagnostic. Parsing never panics or aborts; it
/// accumulates diagnostics and returns partial results, matching the reference parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warning,
    Error,
}

/// A non-fatal parse diagnostic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

/// An `[ext_resource ...]` entry: the id→`res://` path table used to resolve
/// `ExtResource("id")` references and `instance=` scene paths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtResource {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
}

/// A `[sub_resource ...]` entry (inline resource). Sub-resources are not applied over
/// the live-edit wire in v1; nodes referencing them fall back to full-scene reload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubResource {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    #[serde(default)]
    pub properties: Vec<(String, GodotValue)>,
}

/// A `[connection ...]` entry (signal wiring). Carried for completeness; not diffed in v1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub signal: String,
    pub from: String,
    pub to: String,
    pub method: String,
}

/// A single `[node ...]` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneNode {
    pub name: String,
    /// The `type="..."` attribute. Absent for the root of an inherited scene and for
    /// instanced nodes (which carry `instance` instead).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    /// The `parent="..."` attribute, verbatim. `None` marks the scene root; `Some(".")`
    /// is a direct child of the root; `Some("UI/HUD")` is nested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Resolved `res://` scene path when this node is `instance=ExtResource("id")`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_scene: Option<String>,
    /// True when `instance`/`instance_placeholder` referenced something we could not
    /// resolve to a plain `res://` path (e.g. a SubResource) — forces fallback.
    #[serde(default)]
    pub instance_unresolved: bool,
    /// The `index="N"` sibling-order hint, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    /// Property assignments in source order (last-wins on duplicates is applied by the
    /// parser; order is preserved for faithful diffing).
    #[serde(default)]
    pub properties: Vec<(String, GodotValue)>,
}

impl SceneNode {
    /// The node's NodePath relative to the scene root: `"."` for the root, `"Player"`
    /// for a direct child, `"UI/HUD"` for a nested node.
    pub fn relative_path(&self) -> String {
        match self.parent.as_deref() {
            None => ".".to_string(),
            Some("") | Some(".") => self.name.clone(),
            Some(parent) => format!("{parent}/{}", self.name),
        }
    }

    /// The relative NodePath of this node's parent, or `None` if this is the root.
    pub fn parent_path(&self) -> Option<String> {
        match self.parent.as_deref() {
            None => None,
            Some("") => Some(".".to_string()),
            Some(parent) => Some(parent.to_string()),
        }
    }

    /// True when this node is an instanced sub-scene (`instance=ExtResource(...)`).
    pub fn is_instance(&self) -> bool {
        self.instance_scene.is_some() || self.instance_unresolved
    }

    /// Look up a property's parsed value by name.
    pub fn property(&self, name: &str) -> Option<&GodotValue> {
        self.properties
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }
}

/// A fully parsed `.tscn` text scene.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneState {
    #[serde(default)]
    pub nodes: Vec<SceneNode>,
    #[serde(default)]
    pub ext_resources: Vec<ExtResource>,
    #[serde(default)]
    pub sub_resources: Vec<SubResource>,
    #[serde(default)]
    pub connections: Vec<Connection>,
    #[serde(default)]
    pub editable: Vec<String>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

impl SceneState {
    /// The scene root node (the node with no `parent`, else the first node).
    pub fn root(&self) -> Option<&SceneNode> {
        self.nodes
            .iter()
            .find(|node| node.parent.is_none())
            .or_else(|| self.nodes.first())
    }

    /// True if any accumulated diagnostic is an error (a fallback trigger).
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}
