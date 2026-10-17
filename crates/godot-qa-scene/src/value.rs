//! A Godot Variant value parsed from a `.tscn` property, in the subset we can
//! faithfully re-encode for the live-edit wire protocol.
//!
//! Anything outside this subset is represented as [`GodotValue::Unsupported`], which
//! the differ treats as a full-scene-reload fallback trigger rather than silently
//! sending a wrong value. Resource references that resolve to a `res://` path are kept
//! as [`GodotValue::ExtResource`] so the differ can route them through
//! `live_node_prop_res` (path-based) instead of encoding an inline object.

use serde::{Deserialize, Serialize};

/// A parsed `.tscn` property value.
///
/// Math types store `f32` because a stock (single-precision) Godot build encodes them
/// as 32-bit floats on the debugger wire. The int-vs-float distinction is preserved
/// (unlike the reference TypeScript parser) so we encode `INT` vs `FLOAT` correctly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GodotValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    StringName(String),
    NodePath(String),
    Vector2([f32; 2]),
    Vector2i([i32; 2]),
    Vector3([f32; 3]),
    Vector3i([i32; 3]),
    Vector4([f32; 4]),
    Vector4i([i32; 4]),
    Rect2([f32; 4]),
    Rect2i([i32; 4]),
    Plane([f32; 4]),
    Quaternion([f32; 4]),
    Aabb([f32; 6]),
    Basis([f32; 9]),
    Transform2D([f32; 6]),
    Transform3D([f32; 12]),
    Projection([f32; 16]),
    Color([f32; 4]),
    Array(Vec<GodotValue>),
    Dictionary(Vec<(GodotValue, GodotValue)>),
    PackedByteArray(Vec<u8>),
    PackedInt32Array(Vec<i32>),
    PackedInt64Array(Vec<i64>),
    PackedFloat32Array(Vec<f32>),
    PackedFloat64Array(Vec<f64>),
    PackedStringArray(Vec<String>),
    PackedVector2Array(Vec<[f32; 2]>),
    PackedVector3Array(Vec<[f32; 3]>),
    PackedColorArray(Vec<[f32; 4]>),
    /// A resource reference resolved to a `res://` path — applied via `live_node_prop_res`.
    ExtResource(String),
    /// A value we cannot faithfully encode for live edit (SubResource, unknown
    /// constructor, object, etc.). Carries a short reason for diagnostics.
    Unsupported(String),
}

impl GodotValue {
    /// True when this value cannot be sent over the live-edit wire and must force a
    /// full-scene-reload fallback.
    pub fn is_unsupported(&self) -> bool {
        matches!(self, GodotValue::Unsupported(_))
    }

    /// If this value is a resource reference applied by path, return that path.
    pub fn as_ext_resource(&self) -> Option<&str> {
        match self {
            GodotValue::ExtResource(path) => Some(path),
            _ => None,
        }
    }
}
