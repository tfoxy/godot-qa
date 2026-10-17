//! Minimal Godot binary Variant codec for the remote-debugger wire protocol.
//!
//! Only the subset needed to talk to a running Godot game as a debugger host is
//! implemented: we ENCODE a handful of outbound message shapes and DECODE just
//! enough of an inbound frame to read its leading command string. This is NOT a
//! general Variant serializer.
//!
//! Wire format (verified against ../godot-4.5.1-stable/core/io/marshalls.cpp):
//! every framed message is `u32 little-endian length` followed by one encoded
//! Variant. Each Variant begins with a `u32` header whose low byte is the type id
//! (`HEADER_TYPE_MASK = 0xFF`); `HEADER_DATA_FLAG_64 = 1 << 16` marks 64-bit ints.
//! Debugger messages are the 3-element Array `[String command, int thread_id, Array data]`.
//! `thread_id` MUST be `1` (`Thread::MAIN_ID`) or the game silently drops the message.

use godot_qa_scene::GodotValue;

// Variant::Type ordinals (core/variant/variant.h, Godot 4.5.1).
const TYPE_NIL: u32 = 0;
const TYPE_BOOL: u32 = 1;
const TYPE_INT: u32 = 2;
const TYPE_FLOAT: u32 = 3;
const TYPE_STRING: u32 = 4;
const TYPE_VECTOR2: u32 = 5;
const TYPE_VECTOR2I: u32 = 6;
const TYPE_RECT2: u32 = 7;
const TYPE_RECT2I: u32 = 8;
const TYPE_VECTOR3: u32 = 9;
const TYPE_VECTOR3I: u32 = 10;
const TYPE_TRANSFORM2D: u32 = 11;
const TYPE_VECTOR4: u32 = 12;
const TYPE_VECTOR4I: u32 = 13;
const TYPE_PLANE: u32 = 14;
const TYPE_QUATERNION: u32 = 15;
const TYPE_AABB: u32 = 16;
const TYPE_BASIS: u32 = 17;
const TYPE_TRANSFORM3D: u32 = 18;
const TYPE_PROJECTION: u32 = 19;
const TYPE_COLOR: u32 = 20;
const TYPE_STRING_NAME: u32 = 21;
const TYPE_NODE_PATH: u32 = 22;
const TYPE_DICTIONARY: u32 = 27;
const TYPE_ARRAY: u32 = 28;
const TYPE_PACKED_BYTE_ARRAY: u32 = 29;
const TYPE_PACKED_INT32_ARRAY: u32 = 30;
const TYPE_PACKED_INT64_ARRAY: u32 = 31;
const TYPE_PACKED_FLOAT32_ARRAY: u32 = 32;
const TYPE_PACKED_FLOAT64_ARRAY: u32 = 33;
const TYPE_PACKED_STRING_ARRAY: u32 = 34;
const TYPE_PACKED_VECTOR2_ARRAY: u32 = 35;
const TYPE_PACKED_VECTOR3_ARRAY: u32 = 36;
const TYPE_PACKED_COLOR_ARRAY: u32 = 37;

const HEADER_DATA_FLAG_64: u32 = 1 << 16;
const HEADER_TYPE_MASK: u32 = 0xFF;

/// The thread id every host->game message must carry (`Thread::MAIN_ID`).
pub const MAIN_THREAD_ID: i64 = 1;

/// One element of a debugger message's `data` array.
#[derive(Debug, Clone, PartialEq)]
pub enum DataArg {
    Str(String),
    Bool(bool),
    Int(i64),
    /// A single `PackedStringArray` element (e.g. `scene:reload_cached_files`).
    PackedStrArray(Vec<String>),
    /// A `NodePath` (relative or absolute) for the `scene:live_*` structural messages.
    NodePath(String),
    /// A `StringName` (property/method name in `scene:live_node_prop`/`live_node_call`).
    StringName(String),
    /// An arbitrary property value for `scene:live_node_prop`.
    Value(GodotValue),
}

fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_f64(out: &mut Vec<u8>, value: f64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_f32_slice(out: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        write_f32(out, *value);
    }
}

fn write_i32_slice(out: &mut Vec<u8>, values: &[i32]) {
    for value in values {
        write_u32(out, *value as u32);
    }
}

/// Encode a Godot `String` payload: `u32 len` + UTF-8 bytes + zero-pad to 4 bytes.
fn encode_string_payload(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    write_u32(out, bytes.len() as u32);
    out.extend_from_slice(bytes);
    let pad = (4 - (bytes.len() % 4)) % 4;
    out.extend(std::iter::repeat_n(0u8, pad));
}

fn encode_string_variant(out: &mut Vec<u8>, s: &str) {
    write_u32(out, TYPE_STRING);
    encode_string_payload(out, s);
}

fn encode_int_variant(out: &mut Vec<u8>, n: i64) {
    if i32::try_from(n).is_ok() {
        write_u32(out, TYPE_INT);
        write_u32(out, n as u32);
    } else {
        write_u32(out, TYPE_INT | HEADER_DATA_FLAG_64);
        write_u64(out, n as u64);
    }
}

fn encode_bool_variant(out: &mut Vec<u8>, b: bool) {
    write_u32(out, TYPE_BOOL);
    write_u32(out, u32::from(b));
}

/// Encode a `PackedStringArray` Variant. Unlike an `Array` of `String`, each entry's
/// length includes a trailing NUL byte, which is also written, then padded to 4.
fn encode_packed_string_array_variant(out: &mut Vec<u8>, values: &[String]) {
    write_u32(out, TYPE_PACKED_STRING_ARRAY);
    write_u32(out, values.len() as u32);
    for value in values {
        let bytes = value.as_bytes();
        let len_with_nul = bytes.len() + 1;
        write_u32(out, len_with_nul as u32);
        out.extend_from_slice(bytes);
        out.push(0);
        let pad = (4 - (len_with_nul % 4)) % 4;
        out.extend(std::iter::repeat_n(0u8, pad));
    }
}

fn encode_string_name_variant(out: &mut Vec<u8>, s: &str) {
    // STRING_NAME encodes identically to STRING (marshalls.cpp:1529).
    write_u32(out, TYPE_STRING_NAME);
    encode_string_payload(out, s);
}

/// Encode a scalar `FLOAT`. Adaptive width (marshalls.cpp:1376): 32-bit unless the value
/// loses precision as an `f32`, in which case set `HEADER_DATA_FLAG_64` and write 64-bit.
fn encode_float_variant(out: &mut Vec<u8>, value: f64) {
    if (value as f32) as f64 == value {
        write_u32(out, TYPE_FLOAT);
        write_f32(out, value as f32);
    } else {
        write_u32(out, TYPE_FLOAT | HEADER_DATA_FLAG_64);
        write_f64(out, value);
    }
}

/// Encode a `NodePath` Variant (marshalls.cpp:1482). Header word `u32(name_count |
/// 0x80000000)`, `u32 subname_count`, `u32 flags`(bit0=absolute), then each name/subname
/// as `u32 len` + utf8 + pad-to-4 (no trailing NUL). We only generate name segments
/// (no `:subname`s). `"."` parses to a single name `"."` (node_path.cpp), which
/// `get_node` resolves to the node itself.
fn encode_node_path_variant(out: &mut Vec<u8>, path: &str) {
    write_u32(out, TYPE_NODE_PATH);
    let absolute = path.starts_with('/');
    let names: Vec<&str> = path
        .trim_start_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    write_u32(out, (names.len() as u32) | 0x8000_0000);
    write_u32(out, 0); // subname_count
    write_u32(out, u32::from(absolute));
    for name in names {
        encode_string_payload(out, name);
    }
}

/// Encode a `GodotValue` as a full Variant (header + payload). Math types emit 32-bit
/// floats with no 64-bit flag (stock single-precision build); `COLOR` is always float32.
/// `ExtResource`/`Unsupported` never reach here (the differ routes them to
/// `live_node_prop_res` or a full-scene-reload fallback); they encode as `NIL`.
fn encode_value_variant(out: &mut Vec<u8>, value: &GodotValue) {
    match value {
        GodotValue::Null => write_u32(out, TYPE_NIL),
        GodotValue::Bool(b) => encode_bool_variant(out, *b),
        GodotValue::Int(n) => encode_int_variant(out, *n),
        GodotValue::Float(f) => encode_float_variant(out, *f),
        GodotValue::Str(s) => encode_string_variant(out, s),
        GodotValue::StringName(s) => encode_string_name_variant(out, s),
        GodotValue::NodePath(p) => encode_node_path_variant(out, p),
        GodotValue::Vector2(v) => {
            write_u32(out, TYPE_VECTOR2);
            write_f32_slice(out, v);
        }
        GodotValue::Vector2i(v) => {
            write_u32(out, TYPE_VECTOR2I);
            write_i32_slice(out, v);
        }
        GodotValue::Vector3(v) => {
            write_u32(out, TYPE_VECTOR3);
            write_f32_slice(out, v);
        }
        GodotValue::Vector3i(v) => {
            write_u32(out, TYPE_VECTOR3I);
            write_i32_slice(out, v);
        }
        GodotValue::Vector4(v) => {
            write_u32(out, TYPE_VECTOR4);
            write_f32_slice(out, v);
        }
        GodotValue::Vector4i(v) => {
            write_u32(out, TYPE_VECTOR4I);
            write_i32_slice(out, v);
        }
        GodotValue::Rect2(v) => {
            write_u32(out, TYPE_RECT2);
            write_f32_slice(out, v);
        }
        GodotValue::Rect2i(v) => {
            write_u32(out, TYPE_RECT2I);
            write_i32_slice(out, v);
        }
        GodotValue::Plane(v) => {
            write_u32(out, TYPE_PLANE);
            write_f32_slice(out, v);
        }
        GodotValue::Quaternion(v) => {
            write_u32(out, TYPE_QUATERNION);
            write_f32_slice(out, v);
        }
        GodotValue::Aabb(v) => {
            write_u32(out, TYPE_AABB);
            write_f32_slice(out, v);
        }
        GodotValue::Basis(v) => {
            write_u32(out, TYPE_BASIS);
            write_f32_slice(out, v);
        }
        GodotValue::Transform2D(v) => {
            write_u32(out, TYPE_TRANSFORM2D);
            write_f32_slice(out, v);
        }
        GodotValue::Transform3D(v) => {
            write_u32(out, TYPE_TRANSFORM3D);
            write_f32_slice(out, v);
        }
        GodotValue::Projection(v) => {
            write_u32(out, TYPE_PROJECTION);
            write_f32_slice(out, v);
        }
        GodotValue::Color(v) => {
            write_u32(out, TYPE_COLOR);
            write_f32_slice(out, v);
        }
        GodotValue::Array(elements) => {
            write_u32(out, TYPE_ARRAY);
            write_u32(out, elements.len() as u32);
            for element in elements {
                encode_value_variant(out, element);
            }
        }
        GodotValue::Dictionary(pairs) => {
            write_u32(out, TYPE_DICTIONARY);
            write_u32(out, pairs.len() as u32);
            for (key, val) in pairs {
                encode_value_variant(out, key);
                encode_value_variant(out, val);
            }
        }
        GodotValue::PackedByteArray(bytes) => {
            write_u32(out, TYPE_PACKED_BYTE_ARRAY);
            write_u32(out, bytes.len() as u32);
            out.extend_from_slice(bytes);
            let pad = (4 - (bytes.len() % 4)) % 4;
            out.extend(std::iter::repeat_n(0u8, pad));
        }
        GodotValue::PackedInt32Array(values) => {
            write_u32(out, TYPE_PACKED_INT32_ARRAY);
            write_u32(out, values.len() as u32);
            write_i32_slice(out, values);
        }
        GodotValue::PackedInt64Array(values) => {
            write_u32(out, TYPE_PACKED_INT64_ARRAY);
            write_u32(out, values.len() as u32);
            for value in values {
                write_u64(out, *value as u64);
            }
        }
        GodotValue::PackedFloat32Array(values) => {
            write_u32(out, TYPE_PACKED_FLOAT32_ARRAY);
            write_u32(out, values.len() as u32);
            write_f32_slice(out, values);
        }
        GodotValue::PackedFloat64Array(values) => {
            write_u32(out, TYPE_PACKED_FLOAT64_ARRAY);
            write_u32(out, values.len() as u32);
            for value in values {
                write_f64(out, *value);
            }
        }
        GodotValue::PackedStringArray(values) => encode_packed_string_array_variant(out, values),
        GodotValue::PackedVector2Array(values) => {
            write_u32(out, TYPE_PACKED_VECTOR2_ARRAY);
            write_u32(out, values.len() as u32);
            for v in values {
                write_f32_slice(out, v);
            }
        }
        GodotValue::PackedVector3Array(values) => {
            write_u32(out, TYPE_PACKED_VECTOR3_ARRAY);
            write_u32(out, values.len() as u32);
            for v in values {
                write_f32_slice(out, v);
            }
        }
        GodotValue::PackedColorArray(values) => {
            write_u32(out, TYPE_PACKED_COLOR_ARRAY);
            write_u32(out, values.len() as u32);
            for v in values {
                write_f32_slice(out, v);
            }
        }
        GodotValue::ExtResource(_) | GodotValue::Unsupported(_) => {
            debug_assert!(false, "non-wire value reached the encoder: {value:?}");
            write_u32(out, TYPE_NIL);
        }
    }
}

fn encode_data_arg(out: &mut Vec<u8>, arg: &DataArg) {
    match arg {
        DataArg::Str(s) => encode_string_variant(out, s),
        DataArg::Bool(b) => encode_bool_variant(out, *b),
        DataArg::Int(n) => encode_int_variant(out, *n),
        DataArg::PackedStrArray(values) => encode_packed_string_array_variant(out, values),
        DataArg::NodePath(path) => encode_node_path_variant(out, path),
        DataArg::StringName(name) => encode_string_name_variant(out, name),
        DataArg::Value(value) => encode_value_variant(out, value),
    }
}

/// Encode the untyped `Array` Variant body (header + count + elements). An untyped
/// array writes no container-type bytes.
fn encode_array_variant(out: &mut Vec<u8>, elements: &[DataArg]) {
    write_u32(out, TYPE_ARRAY);
    write_u32(out, elements.len() as u32);
    for element in elements {
        encode_data_arg(out, element);
    }
}

/// Encode a full framed debugger message ready to write to the socket:
/// `u32 LE length` + encoded `Array[String command, int thread_id, Array data]`.
pub fn encode_message(command: &str, thread_id: i64, data: &[DataArg]) -> Vec<u8> {
    let mut body = Vec::new();
    // Outer message array: [command, thread_id, data].
    write_u32(&mut body, TYPE_ARRAY);
    write_u32(&mut body, 3);
    encode_string_variant(&mut body, command);
    encode_int_variant(&mut body, thread_id);
    encode_array_variant(&mut body, data);

    let mut frame = Vec::with_capacity(body.len() + 4);
    write_u32(&mut frame, body.len() as u32);
    frame.extend_from_slice(&body);
    frame
}

/// `["reload_scripts", 1, [paths...]]` — soft (state-preserving) reload of specific scripts.
pub fn message_reload_scripts(paths: &[String]) -> Vec<u8> {
    let data: Vec<DataArg> = paths.iter().cloned().map(DataArg::Str).collect();
    encode_message("reload_scripts", MAIN_THREAD_ID, &data)
}

/// `["reload_all_scripts", 1, []]`.
pub fn message_reload_all_scripts() -> Vec<u8> {
    encode_message("reload_all_scripts", MAIN_THREAD_ID, &[])
}

/// `["scene:reload_cached_files", 1, [PackedStringArray(paths)]]` — live resource reload.
pub fn message_reload_cached_files(paths: &[String]) -> Vec<u8> {
    encode_message(
        "scene:reload_cached_files",
        MAIN_THREAD_ID,
        &[DataArg::PackedStrArray(paths.to_vec())],
    )
}

/// `["set_ignore_error_breaks", 1, [value]]` — keep the game from blocking on script errors.
pub fn message_set_ignore_error_breaks(value: bool) -> Vec<u8> {
    encode_message(
        "set_ignore_error_breaks",
        MAIN_THREAD_ID,
        &[DataArg::Bool(value)],
    )
}

/// `["set_skip_breakpoints", 1, [value]]`.
pub fn message_set_skip_breakpoints(value: bool) -> Vec<u8> {
    encode_message(
        "set_skip_breakpoints",
        MAIN_THREAD_ID,
        &[DataArg::Bool(value)],
    )
}

/// `["continue", 1, []]` — resume a game that broke into the debugger (data ignored by the game).
pub fn message_continue() -> Vec<u8> {
    encode_message("continue", MAIN_THREAD_ID, &[])
}

/// `["scene:request_scene_tree", 1, []]` — ask the game to reply with its live tree.
pub fn message_request_scene_tree() -> Vec<u8> {
    encode_message("scene:request_scene_tree", MAIN_THREAD_ID, &[])
}

/// `["scene:live_set_root", 1, [NodePath base, String scene_path]]` — key subsequent
/// live edits to `scene_path`, applied under `base` in every matching live instance.
pub fn message_live_set_root(base: &str, scene_path: &str) -> Vec<u8> {
    encode_message(
        "scene:live_set_root",
        MAIN_THREAD_ID,
        &[
            DataArg::NodePath(base.to_string()),
            DataArg::Str(scene_path.to_string()),
        ],
    )
}

/// `["scene:live_node_path", 1, [NodePath path, int id]]` — register `id` → `path` so
/// later property/method messages can reference the node by the interned id.
pub fn message_live_node_path(path: &str, id: i64) -> Vec<u8> {
    encode_message(
        "scene:live_node_path",
        MAIN_THREAD_ID,
        &[DataArg::NodePath(path.to_string()), DataArg::Int(id)],
    )
}

/// `["scene:live_create_node", 1, [NodePath parent, String type, String name]]`.
pub fn message_live_create_node(parent: &str, type_name: &str, name: &str) -> Vec<u8> {
    encode_message(
        "scene:live_create_node",
        MAIN_THREAD_ID,
        &[
            DataArg::NodePath(parent.to_string()),
            DataArg::Str(type_name.to_string()),
            DataArg::Str(name.to_string()),
        ],
    )
}

/// `["scene:live_instantiate_node", 1, [NodePath parent, String scene_path, String name]]`.
pub fn message_live_instantiate_node(parent: &str, scene_path: &str, name: &str) -> Vec<u8> {
    encode_message(
        "scene:live_instantiate_node",
        MAIN_THREAD_ID,
        &[
            DataArg::NodePath(parent.to_string()),
            DataArg::Str(scene_path.to_string()),
            DataArg::Str(name.to_string()),
        ],
    )
}

/// `["scene:live_remove_node", 1, [NodePath at]]`.
pub fn message_live_remove_node(path: &str) -> Vec<u8> {
    encode_message(
        "scene:live_remove_node",
        MAIN_THREAD_ID,
        &[DataArg::NodePath(path.to_string())],
    )
}

/// `["scene:live_reparent_node", 1, [NodePath at, NodePath new_parent, String new_name, int pos]]`.
pub fn message_live_reparent_node(
    path: &str,
    new_parent: &str,
    new_name: &str,
    pos: i64,
) -> Vec<u8> {
    encode_message(
        "scene:live_reparent_node",
        MAIN_THREAD_ID,
        &[
            DataArg::NodePath(path.to_string()),
            DataArg::NodePath(new_parent.to_string()),
            DataArg::Str(new_name.to_string()),
            DataArg::Int(pos),
        ],
    )
}

/// `["scene:live_duplicate_node", 1, [NodePath at, String new_name]]`.
pub fn message_live_duplicate_node(path: &str, new_name: &str) -> Vec<u8> {
    encode_message(
        "scene:live_duplicate_node",
        MAIN_THREAD_ID,
        &[
            DataArg::NodePath(path.to_string()),
            DataArg::Str(new_name.to_string()),
        ],
    )
}

/// `["scene:live_node_prop", 1, [int id, StringName prop, Variant value]]` — set a node
/// property (by interned path id) to an encodable Variant value.
pub fn message_live_node_prop(id: i64, prop: &str, value: &GodotValue) -> Vec<u8> {
    encode_message(
        "scene:live_node_prop",
        MAIN_THREAD_ID,
        &[
            DataArg::Int(id),
            DataArg::StringName(prop.to_string()),
            DataArg::Value(value.clone()),
        ],
    )
}

/// `["scene:live_node_prop_res", 1, [int id, StringName prop, String res_path]]` — set a
/// node property (by interned path id) to a resource loaded from `res_path`.
pub fn message_live_node_prop_res(id: i64, prop: &str, res_path: &str) -> Vec<u8> {
    encode_message(
        "scene:live_node_prop_res",
        MAIN_THREAD_ID,
        &[
            DataArg::Int(id),
            DataArg::StringName(prop.to_string()),
            DataArg::Str(res_path.to_string()),
        ],
    )
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes(slice.try_into().ok()?))
}

/// Decode only the leading command string of an inbound frame body (the bytes AFTER
/// the length prefix). Returns `None` if the frame is not the expected
/// `Array[String, ...]` shape. Everything after the command string is ignored.
pub fn peek_command(body: &[u8]) -> Option<String> {
    let array_header = read_u32(body, 0)?;
    if array_header & HEADER_TYPE_MASK != TYPE_ARRAY {
        return None;
    }
    let count = read_u32(body, 4)?;
    if count == 0 {
        return None;
    }
    let elem_header = read_u32(body, 8)?;
    if elem_header & HEADER_TYPE_MASK != TYPE_STRING {
        return None;
    }
    let len = read_u32(body, 12)? as usize;
    let start = 16;
    let string_bytes = body.get(start..start + len)?;
    String::from_utf8(string_bytes.to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_body(frame: &[u8]) -> &[u8] {
        let len = u32::from_le_bytes(frame[0..4].try_into().unwrap()) as usize;
        assert_eq!(len, frame.len() - 4, "length prefix must match body length");
        &frame[4..]
    }

    #[test]
    fn reload_scripts_round_trips_command() {
        let frame = message_reload_scripts(&["res://player.gd".to_string()]);
        assert_eq!(
            peek_command(frame_body(&frame)).as_deref(),
            Some("reload_scripts")
        );
    }

    #[test]
    fn reload_all_scripts_has_empty_data() {
        let frame = message_reload_all_scripts();
        assert_eq!(
            peek_command(frame_body(&frame)).as_deref(),
            Some("reload_all_scripts")
        );
    }

    #[test]
    fn reload_cached_files_round_trips_command() {
        let frame = message_reload_cached_files(&["res://ui/theme.tres".to_string()]);
        assert_eq!(
            peek_command(frame_body(&frame)).as_deref(),
            Some("scene:reload_cached_files")
        );
    }

    #[test]
    fn safety_frames_encode_expected_commands() {
        assert_eq!(
            peek_command(frame_body(&message_set_ignore_error_breaks(true))).as_deref(),
            Some("set_ignore_error_breaks")
        );
        assert_eq!(
            peek_command(frame_body(&message_set_skip_breakpoints(true))).as_deref(),
            Some("set_skip_breakpoints")
        );
        assert_eq!(
            peek_command(frame_body(&message_continue())).as_deref(),
            Some("continue")
        );
    }

    #[test]
    fn string_payload_is_padded_to_four_bytes() {
        // "ab" -> u32 len(2) + 2 bytes + 2 pad = 8 bytes.
        let mut out = Vec::new();
        encode_string_payload(&mut out, "ab");
        assert_eq!(out.len(), 8);
        assert_eq!(&out[0..4], &2u32.to_le_bytes());
        assert_eq!(&out[4..6], b"ab");
        assert_eq!(&out[6..8], &[0, 0]);
    }

    #[test]
    fn packed_string_array_writes_nul_and_padding() {
        // A single "a" -> count(1) + [ len(2 incl NUL) + 'a' + NUL + 2 pad ].
        let mut out = Vec::new();
        encode_packed_string_array_variant(&mut out, &["a".to_string()]);
        // header(4) + count(4) + len(4) + 'a'+NUL(2) + pad(2) = 16
        assert_eq!(out.len(), 16);
        assert_eq!(&out[0..4], &TYPE_PACKED_STRING_ARRAY.to_le_bytes());
        assert_eq!(&out[4..8], &1u32.to_le_bytes());
        assert_eq!(&out[8..12], &2u32.to_le_bytes());
        assert_eq!(&out[12..14], b"a\0");
        assert_eq!(&out[14..16], &[0, 0]);
    }

    #[test]
    fn encoded_message_body_length_is_four_aligned() {
        // Every Variant in this subset is 4-aligned, so bodies must be too.
        for frame in [
            message_reload_scripts(&["res://a.gd".to_string(), "res://bb.gd".to_string()]),
            message_reload_all_scripts(),
            message_reload_cached_files(&["res://x.tres".to_string()]),
            message_continue(),
        ] {
            let body = frame_body(&frame);
            assert_eq!(body.len() % 4, 0, "body must be 4-byte aligned");
        }
    }

    #[test]
    fn peek_command_rejects_non_array_frames() {
        // A bare string variant, not an array.
        let mut body = Vec::new();
        encode_string_variant(&mut body, "not-an-array");
        assert_eq!(peek_command(&body), None);
    }

    #[test]
    fn peek_command_handles_truncated_frames() {
        assert_eq!(peek_command(&[]), None);
        assert_eq!(peek_command(&[28, 0, 0]), None);
    }

    fn value_bytes(value: &GodotValue) -> Vec<u8> {
        let mut out = Vec::new();
        encode_value_variant(&mut out, value);
        out
    }

    #[test]
    fn node_path_encodes_names_len_and_flags() {
        let mut out = Vec::new();
        encode_node_path_variant(&mut out, "UI/HUD");
        let mut expected = Vec::new();
        expected.extend_from_slice(&TYPE_NODE_PATH.to_le_bytes());
        expected.extend_from_slice(&(2u32 | 0x8000_0000).to_le_bytes()); // 2 names
        expected.extend_from_slice(&0u32.to_le_bytes()); // 0 subnames
        expected.extend_from_slice(&0u32.to_le_bytes()); // not absolute
        expected.extend_from_slice(&2u32.to_le_bytes()); // "UI"
        expected.extend_from_slice(b"UI\0\0");
        expected.extend_from_slice(&3u32.to_le_bytes()); // "HUD"
        expected.extend_from_slice(b"HUD\0");
        assert_eq!(out, expected);
    }

    #[test]
    fn node_path_dot_is_single_self_name() {
        let mut out = Vec::new();
        encode_node_path_variant(&mut out, ".");
        // name_count = 1, the name is ".".
        assert_eq!(&out[0..4], &TYPE_NODE_PATH.to_le_bytes());
        assert_eq!(&out[4..8], &(1u32 | 0x8000_0000).to_le_bytes());
        assert_eq!(&out[8..12], &0u32.to_le_bytes()); // subnames
        assert_eq!(&out[12..16], &0u32.to_le_bytes()); // relative
        assert_eq!(&out[16..20], &1u32.to_le_bytes()); // len 1
        assert_eq!(&out[20..24], b".\0\0\0");
    }

    #[test]
    fn node_path_absolute_sets_flag() {
        let mut out = Vec::new();
        encode_node_path_variant(&mut out, "/root");
        assert_eq!(&out[4..8], &(1u32 | 0x8000_0000).to_le_bytes()); // one name "root"
        assert_eq!(&out[12..16], &1u32.to_le_bytes()); // absolute flag
        assert_eq!(&out[16..20], &4u32.to_le_bytes()); // len 4
        assert_eq!(&out[20..24], b"root");
    }

    #[test]
    fn vector2_and_color_are_float32() {
        let mut expected = Vec::new();
        expected.extend_from_slice(&TYPE_VECTOR2.to_le_bytes());
        expected.extend_from_slice(&1.0f32.to_le_bytes());
        expected.extend_from_slice(&2.0f32.to_le_bytes());
        assert_eq!(value_bytes(&GodotValue::Vector2([1.0, 2.0])), expected);

        let mut expected_color = Vec::new();
        expected_color.extend_from_slice(&TYPE_COLOR.to_le_bytes());
        for component in [1.0f32, 0.0, 0.0, 1.0] {
            expected_color.extend_from_slice(&component.to_le_bytes());
        }
        assert_eq!(
            value_bytes(&GodotValue::Color([1.0, 0.0, 0.0, 1.0])),
            expected_color
        );
    }

    #[test]
    fn float_is_adaptive_width() {
        // 1.5 round-trips through f32 → 32-bit, no flag.
        let short = value_bytes(&GodotValue::Float(1.5));
        assert_eq!(&short[0..4], &TYPE_FLOAT.to_le_bytes());
        assert_eq!(short.len(), 8);
        assert_eq!(&short[4..8], &1.5f32.to_le_bytes());

        // 0.1 loses precision as f32 → 64-bit flag + 8-byte payload.
        let long = value_bytes(&GodotValue::Float(0.1));
        assert_eq!(
            &long[0..4],
            &(TYPE_FLOAT | HEADER_DATA_FLAG_64).to_le_bytes()
        );
        assert_eq!(long.len(), 12);
        assert_eq!(&long[4..12], &0.1f64.to_le_bytes());
    }

    #[test]
    fn int_value_matches_int_variant() {
        assert_eq!(value_bytes(&GodotValue::Int(7)), {
            let mut out = Vec::new();
            encode_int_variant(&mut out, 7);
            out
        });
    }

    #[test]
    fn live_messages_round_trip_prefixed_commands() {
        let cases: [(Vec<u8>, &str); 8] = [
            (message_request_scene_tree(), "scene:request_scene_tree"),
            (
                message_live_set_root("/root", "res://main.tscn"),
                "scene:live_set_root",
            ),
            (message_live_node_path("UI/HUD", 3), "scene:live_node_path"),
            (
                message_live_create_node(".", "Sprite2D", "Bullet"),
                "scene:live_create_node",
            ),
            (
                message_live_instantiate_node(".", "res://enemy.tscn", "Enemy"),
                "scene:live_instantiate_node",
            ),
            (message_live_remove_node("UI/Old"), "scene:live_remove_node"),
            (
                message_live_reparent_node("Score", "UI", "Score", -1),
                "scene:live_reparent_node",
            ),
            (
                message_live_node_prop(3, "visible", &GodotValue::Bool(true)),
                "scene:live_node_prop",
            ),
        ];
        for (frame, command) in cases {
            assert_eq!(peek_command(frame_body(&frame)).as_deref(), Some(command));
            assert_eq!(frame_body(&frame).len() % 4, 0, "body must be 4-aligned");
        }
    }

    #[test]
    fn live_node_prop_res_round_trips() {
        let frame = message_live_node_prop_res(3, "texture", "res://icon.png");
        assert_eq!(
            peek_command(frame_body(&frame)).as_deref(),
            Some("scene:live_node_prop_res")
        );
    }
}
