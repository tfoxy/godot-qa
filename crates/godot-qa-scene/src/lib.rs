//! Parsing and structural diffing of Godot `.tscn` text scenes for engine-faithful
//! hot reload.
//!
//! This crate is pure (no I/O, no wire codec): it parses a `.tscn` into a [`SceneState`],
//! and diffs two versions into a [`SceneDiff`] of [`SceneEditOp`]s that map 1:1 onto
//! Godot's `scene:live_*` debugger messages. The `godot-qa-debug` sidecar owns the
//! snapshot baseline, encodes the ops to wire frames, and sends them.

pub mod diff;
pub mod model;
pub mod ops;
pub mod parser;
pub mod value;

pub use diff::diff;
pub use model::{
    Connection, Diagnostic, ExtResource, SceneNode, SceneState, Severity, SubResource,
};
pub use ops::{SceneDiff, SceneEditOp};
pub use parser::parse_scene;
pub use value::GodotValue;
