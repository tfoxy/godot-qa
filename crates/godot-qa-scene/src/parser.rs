//! `.tscn` (Godot text scene) parser.
//!
//! Uses a line loop that classifies
//! `[section ...]` headers vs `key = value` assignments, an incremental multi-line
//! logical-line collector (so multi-line arrays/dicts/strings parse), and a
//! recursive-descent value parser producing [`GodotValue`]. Parsing never panics; it
//! accumulates [`Diagnostic`]s and returns partial results.

use std::collections::HashMap;

use crate::model::{
    Connection, Diagnostic, ExtResource, SceneNode, SceneState, Severity, SubResource,
};
use crate::value::GodotValue;

/// Parse `.tscn` text into a [`SceneState`]. Accumulates non-fatal diagnostics rather
/// than failing.
pub fn parse_scene(text: &str) -> SceneState {
    // Split on `\r?\n`, dropping a leading BOM (mirrors the reference parser).
    let normalized = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = normalized
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();

    // Collapse physical lines into logical lines (merging across unbalanced
    // quotes/brackets/braces/parens), recording the 1-based start line of each.
    let mut logical: Vec<(usize, String)> = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let (raw, end) = collect_logical_line(&lines, index);
        logical.push((index + 1, raw));
        index = end + 1;
    }

    // Strip line comments + trim, dropping now-empty lines.
    let processed: Vec<(usize, String)> = logical
        .iter()
        .filter_map(|(line_number, raw)| {
            let stripped = strip_line_comment(raw);
            let trimmed = stripped.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some((*line_number, trimmed.to_string()))
            }
        })
        .collect();

    // Pre-pass: build the ext-resource id -> `res://` path table so `ExtResource("id")`
    // (and node `instance=`) references resolve regardless of section ordering.
    let mut ext_table: HashMap<String, String> = HashMap::new();
    for (line_number, line) in &processed {
        if !(line.starts_with('[') && line.ends_with(']')) {
            continue;
        }
        let inner = &line[1..line.len() - 1];
        let parts = split_header_parts(inner);
        if parts.first().map(String::as_str) != Some("ext_resource") {
            continue;
        }
        let attr_source = if parts.len() > 1 {
            parts[1..].join(" ")
        } else {
            String::new()
        };
        let mut throwaway = Vec::new();
        let attributes = parse_header_attributes(
            &attr_source,
            &mut throwaway,
            Some(*line_number),
            &HashMap::new(),
        );
        if let (Some(id), Some(path)) = (
            string_attribute(&attributes, "id"),
            string_attribute(&attributes, "path"),
        ) {
            ext_table.insert(id, path);
        }
    }

    // Main pass.
    let mut state = SceneState::default();
    let mut cursor = Cursor::Ignore;
    for (line_number, line) in &processed {
        if line.starts_with('[') && line.ends_with(']') {
            let inner = &line[1..line.len() - 1];
            cursor = parse_section_header(inner, *line_number, &mut state, &ext_table);
            continue;
        }
        match split_assignment(line) {
            Some((key, value_source)) => {
                let value = parse_godot_value(
                    &value_source,
                    &mut state.diagnostics,
                    Some(*line_number),
                    &ext_table,
                );
                match cursor {
                    Cursor::Node(node_index) => {
                        set_property(&mut state.nodes[node_index].properties, key, value);
                    }
                    Cursor::SubResource(sub_index) => {
                        set_property(&mut state.sub_resources[sub_index].properties, key, value);
                    }
                    Cursor::Ignore => {}
                }
            }
            None => state.diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                code: "godot_parser_unsupported_line".to_string(),
                message: "Unsupported Godot text line.".to_string(),
                line: Some(*line_number),
            }),
        }
    }

    state
}

/// Where subsequent `key = value` lines attach.
enum Cursor {
    Node(usize),
    SubResource(usize),
    /// Headers whose bodies we don't retain (gd_scene, ext_resource, connection, …).
    Ignore,
}

fn parse_section_header(
    inner: &str,
    line: usize,
    state: &mut SceneState,
    ext_table: &HashMap<String, String>,
) -> Cursor {
    let parts = split_header_parts(inner);
    let section = parts.first().cloned().unwrap_or_default();
    let attr_source = if parts.len() > 1 {
        parts[1..].join(" ")
    } else {
        String::new()
    };
    let attributes =
        parse_header_attributes(&attr_source, &mut state.diagnostics, Some(line), ext_table);

    match section.as_str() {
        "gd_scene" | "gd_resource" | "resource" => Cursor::Ignore,
        "ext_resource" => {
            state.ext_resources.push(ExtResource {
                id: string_attribute(&attributes, "id").unwrap_or_default(),
                type_name: string_attribute(&attributes, "type"),
                path: string_attribute(&attributes, "path"),
                uid: string_attribute(&attributes, "uid"),
            });
            Cursor::Ignore
        }
        "sub_resource" => {
            state.sub_resources.push(SubResource {
                id: string_attribute(&attributes, "id").unwrap_or_default(),
                type_name: string_attribute(&attributes, "type"),
                properties: Vec::new(),
            });
            Cursor::SubResource(state.sub_resources.len() - 1)
        }
        "node" => {
            let mut node = SceneNode {
                name: string_attribute(&attributes, "name").unwrap_or_default(),
                type_name: string_attribute(&attributes, "type"),
                parent: string_attribute(&attributes, "parent"),
                instance_scene: None,
                instance_unresolved: false,
                index: node_index_attribute(&attributes),
                groups: groups_attribute(&attributes),
                properties: Vec::new(),
            };
            match attribute(&attributes, "instance") {
                Some(GodotValue::ExtResource(path)) => node.instance_scene = Some(path.clone()),
                // `instance=SubResource(...)` (Unsupported) or any other unexpected shape
                // cannot be resolved to a plain `res://` path.
                Some(_) => node.instance_unresolved = true,
                None => {}
            }
            if attribute(&attributes, "instance_placeholder").is_some() {
                node.instance_unresolved = true;
            }
            state.nodes.push(node);
            Cursor::Node(state.nodes.len() - 1)
        }
        "connection" => {
            state.connections.push(Connection {
                signal: string_attribute(&attributes, "signal").unwrap_or_default(),
                from: string_attribute(&attributes, "from").unwrap_or_default(),
                to: string_attribute(&attributes, "to").unwrap_or_default(),
                method: string_attribute(&attributes, "method").unwrap_or_default(),
            });
            Cursor::Ignore
        }
        "editable" => {
            if let Some(path) = string_attribute(&attributes, "path") {
                state.editable.push(path);
            }
            Cursor::Ignore
        }
        other => {
            state.diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                code: "godot_parser_unknown_section".to_string(),
                message: format!("Unknown Godot text section: {other}"),
                line: Some(line),
            });
            Cursor::Ignore
        }
    }
}

/// Insert or update a property, preserving first-seen source order (last value wins on
/// a duplicate key).
fn set_property(properties: &mut Vec<(String, GodotValue)>, key: String, value: GodotValue) {
    if let Some(entry) = properties.iter_mut().find(|(existing, _)| *existing == key) {
        entry.1 = value;
    } else {
        properties.push((key, value));
    }
}

// ---------------------------------------------------------------------------
// Header attribute helpers
// ---------------------------------------------------------------------------

fn attribute<'a>(attributes: &'a [(String, GodotValue)], key: &str) -> Option<&'a GodotValue> {
    attributes
        .iter()
        .find(|(existing, _)| existing == key)
        .map(|(_, value)| value)
}

fn string_attribute(attributes: &[(String, GodotValue)], key: &str) -> Option<String> {
    match attribute(attributes, key) {
        Some(GodotValue::Str(value)) => Some(value.clone()),
        _ => None,
    }
}

// Godot serializes a node header `index` as a quoted string (`index="3"`), so accept a
// numeric string as well as a bare integer.
fn node_index_attribute(attributes: &[(String, GodotValue)]) -> Option<i64> {
    match attribute(attributes, "index") {
        Some(GodotValue::Int(value)) => Some(*value),
        Some(GodotValue::Str(value)) => value.parse::<i64>().ok(),
        _ => None,
    }
}

fn groups_attribute(attributes: &[(String, GodotValue)]) -> Vec<String> {
    match attribute(attributes, "groups") {
        Some(GodotValue::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                GodotValue::Str(value) => Some(value.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Split a header body into whitespace-separated parts, treating quoted spans as opaque
/// (so `name="a b"` is one part).
fn split_header_parts(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut parts: Vec<String> = Vec::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut start = 0;
    for (position, &current) in chars.iter().enumerate() {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if current == '\\' {
                escaped = true;
            } else if current == active {
                quote = None;
            }
            continue;
        }
        if current == '"' || current == '\'' {
            quote = Some(current);
        } else if current.is_whitespace() {
            if position > start {
                parts.push(chars[start..position].iter().collect());
            }
            start = position + 1;
        }
    }
    if start < chars.len() {
        parts.push(chars[start..].iter().collect());
    }
    parts
}

/// Parse `key=value key=value …` header attributes. Mirrors the reference parser's
/// `([A-Za-z0-9_./-]+)=` scan: each value spans up to the next attribute key (or the
/// end), and is parsed with the same value grammar as properties.
fn parse_header_attributes(
    source: &str,
    diagnostics: &mut Vec<Diagnostic>,
    line: Option<usize>,
    ext_table: &HashMap<String, String>,
) -> Vec<(String, GodotValue)> {
    let chars: Vec<char> = source.chars().collect();
    // Each match: (key, key_start, value_start).
    let mut matches: Vec<(String, usize, usize)> = Vec::new();
    let mut position = 0;
    while position < chars.len() {
        if is_key_char(chars[position]) {
            let start = position;
            let mut end = position;
            while end < chars.len() && is_key_char(chars[end]) {
                end += 1;
            }
            if end < chars.len() && chars[end] == '=' {
                matches.push((chars[start..end].iter().collect(), start, end + 1));
                position = end + 1;
            } else {
                position = end;
            }
        } else {
            position += 1;
        }
    }

    let mut attributes: Vec<(String, GodotValue)> = Vec::new();
    for (match_index, (key, _, value_start)) in matches.iter().enumerate() {
        let value_end = matches
            .get(match_index + 1)
            .map(|(_, next_start, _)| *next_start)
            .unwrap_or(chars.len());
        let value_source: String = chars[*value_start..value_end].iter().collect();
        let value = parse_godot_value(value_source.trim(), diagnostics, line, ext_table);
        set_property(&mut attributes, key.clone(), value);
    }
    attributes
}

fn is_key_char(value: char) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, '_' | '.' | '/' | '-')
}

// ---------------------------------------------------------------------------
// Line/logical-line utilities
// ---------------------------------------------------------------------------

fn split_assignment(line: &str) -> Option<(String, String)> {
    let index = line.find('=')?;
    Some((
        line[..index].trim().to_string(),
        line[index + 1..].trim().to_string(),
    ))
}

fn strip_line_comment(line: &str) -> String {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut result = String::new();
    for current in line.chars() {
        if let Some(active) = quote {
            result.push(current);
            if escaped {
                escaped = false;
            } else if current == '\\' {
                escaped = true;
            } else if current == active {
                quote = None;
            }
            continue;
        }
        if current == '"' || current == '\'' {
            quote = Some(current);
            result.push(current);
        } else if current == ';' {
            break;
        } else {
            result.push(current);
        }
    }
    result
}

#[derive(Default)]
struct OpenValueState {
    quote: Option<char>,
    escaped: bool,
    in_comment: bool,
    square_depth: i32,
    curly_depth: i32,
    paren_depth: i32,
}

fn is_open_value(state: &OpenValueState) -> bool {
    state.quote.is_some()
        || state.square_depth > 0
        || state.curly_depth > 0
        || state.paren_depth > 0
}

fn scan_open_value(source: &str, state: &mut OpenValueState) {
    for current in source.chars() {
        if state.in_comment {
            if current == '\n' {
                state.in_comment = false;
            }
            continue;
        }
        if let Some(active) = state.quote {
            if state.escaped {
                state.escaped = false;
            } else if current == '\\' {
                state.escaped = true;
            } else if current == active {
                state.quote = None;
            }
            continue;
        }
        match current {
            '"' | '\'' => state.quote = Some(current),
            ';' => state.in_comment = true,
            '[' => state.square_depth += 1,
            ']' => state.square_depth = (state.square_depth - 1).max(0),
            '{' => state.curly_depth += 1,
            '}' => state.curly_depth = (state.curly_depth - 1).max(0),
            '(' => state.paren_depth += 1,
            ')' => state.paren_depth = (state.paren_depth - 1).max(0),
            _ => {}
        }
    }
}

/// Merge physical lines into one logical line while any quote/bracket/brace/paren stays
/// unbalanced. Returns the joined text and the index of the last physical line consumed.
fn collect_logical_line(lines: &[&str], start: usize) -> (String, usize) {
    let mut state = OpenValueState::default();
    let first = lines.get(start).copied().unwrap_or("");
    let mut parts: Vec<&str> = vec![first];
    scan_open_value(first, &mut state);
    let mut end = start;
    while is_open_value(&state) && end + 1 < lines.len() {
        end += 1;
        let line = lines[end];
        parts.push(line);
        scan_open_value(&format!("\n{line}"), &mut state);
    }
    (parts.join("\n"), end)
}

// ---------------------------------------------------------------------------
// Value grammar (recursive descent)
// ---------------------------------------------------------------------------

fn parse_godot_value(
    source: &str,
    diagnostics: &mut Vec<Diagnostic>,
    line: Option<usize>,
    ext_table: &HashMap<String, String>,
) -> GodotValue {
    let mut parser = ValueParser::new(source, diagnostics, line, ext_table);
    let value = parser.parse_value();
    parser.skip_trivia();
    if !parser.done() {
        let rest = parser.remaining();
        parser.warn(
            "godot_parser_trailing_value_text",
            format!("Trailing text after Godot value: {rest}"),
        );
    }
    value
}

struct ValueParser<'a> {
    chars: Vec<char>,
    position: usize,
    diagnostics: &'a mut Vec<Diagnostic>,
    line: Option<usize>,
    ext_table: &'a HashMap<String, String>,
}

impl<'a> ValueParser<'a> {
    fn new(
        source: &str,
        diagnostics: &'a mut Vec<Diagnostic>,
        line: Option<usize>,
        ext_table: &'a HashMap<String, String>,
    ) -> Self {
        ValueParser {
            chars: source.chars().collect(),
            position: 0,
            diagnostics,
            line,
            ext_table,
        }
    }

    fn parse_value(&mut self) -> GodotValue {
        self.skip_trivia();
        let current = self.peek();
        if current == '"' || current == '\'' {
            return GodotValue::Str(self.parse_string());
        }
        if current == '[' {
            return self.parse_array();
        }
        if current == '{' {
            return self.parse_dictionary();
        }
        if current == '&' {
            return self.parse_string_name();
        }
        if current == '-' || current == '+' || current == '.' || current.is_ascii_digit() {
            return self.parse_number();
        }
        let ident = self.parse_identifier();
        if ident.is_empty() {
            self.warn(
                "godot_parser_empty_value",
                "Expected a Godot value.".to_string(),
            );
            // Consume the character no value rule matched. Without this, the
            // array/dictionary/call-argument loops that retry parse_value at
            // the same position never advance and grow their element vectors
            // unboundedly — a 40-byte input like `a = Foo([)` could allocate
            // gigabytes before dying.
            if !self.done() {
                self.position += 1;
            }
            return GodotValue::Null;
        }
        match ident.as_str() {
            "true" => return GodotValue::Bool(true),
            "false" => return GodotValue::Bool(false),
            "null" | "nil" => return GodotValue::Null,
            _ => {}
        }
        self.skip_trivia();
        if self.peek() == '(' {
            return self.parse_call(&ident);
        }
        GodotValue::Str(ident)
    }

    fn skip_trivia(&mut self) {
        while !self.done() && self.peek().is_whitespace() {
            self.position += 1;
        }
    }

    fn done(&self) -> bool {
        self.position >= self.chars.len()
    }

    fn peek(&self) -> char {
        self.chars.get(self.position).copied().unwrap_or('\0')
    }

    fn remaining(&self) -> String {
        self.chars[self.position..].iter().collect()
    }

    fn warn(&mut self, code: &str, message: String) {
        self.diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            code: code.to_string(),
            message,
            line: self.line,
        });
    }

    fn parse_string(&mut self) -> String {
        let quote = self.peek();
        self.position += 1;
        let mut result = String::new();
        let mut escaped = false;
        while !self.done() {
            let current = self.peek();
            self.position += 1;
            if escaped {
                result.push(decode_escape(current));
                escaped = false;
            } else if current == '\\' {
                escaped = true;
            } else if current == quote {
                return result;
            } else {
                result.push(current);
            }
        }
        self.warn(
            "godot_parser_unterminated_string",
            "Unterminated Godot string.".to_string(),
        );
        result
    }

    fn parse_array(&mut self) -> GodotValue {
        self.position += 1;
        let mut values: Vec<GodotValue> = Vec::new();
        while !self.done() {
            self.skip_trivia();
            if self.peek() == ']' {
                self.position += 1;
                return GodotValue::Array(values);
            }
            values.push(self.parse_value());
            self.skip_trivia();
            if self.peek() == ',' {
                self.position += 1;
            }
        }
        self.warn(
            "godot_parser_unterminated_array",
            "Unterminated Godot array.".to_string(),
        );
        GodotValue::Array(values)
    }

    fn parse_dictionary(&mut self) -> GodotValue {
        self.position += 1;
        let mut entries: Vec<(GodotValue, GodotValue)> = Vec::new();
        while !self.done() {
            self.skip_trivia();
            if self.peek() == '}' {
                self.position += 1;
                return GodotValue::Dictionary(entries);
            }
            let key = self.parse_value();
            self.skip_trivia();
            if self.peek() != ':' {
                self.warn(
                    "godot_parser_dictionary_colon_missing",
                    "Expected ':' in Godot dictionary.".to_string(),
                );
                return GodotValue::Dictionary(entries);
            }
            self.position += 1;
            let value = self.parse_value();
            entries.push((dictionary_key(key), value));
            self.skip_trivia();
            if self.peek() == ',' {
                self.position += 1;
            }
        }
        self.warn(
            "godot_parser_unterminated_dictionary",
            "Unterminated Godot dictionary.".to_string(),
        );
        GodotValue::Dictionary(entries)
    }

    fn parse_number(&mut self) -> GodotValue {
        let start = self.position;
        let mut is_float = false;
        if matches!(self.peek(), '+' | '-') {
            self.position += 1;
        }
        while self.peek().is_ascii_digit() {
            self.position += 1;
        }
        if self.peek() == '.' {
            is_float = true;
            self.position += 1;
            while self.peek().is_ascii_digit() {
                self.position += 1;
            }
        }
        if matches!(self.peek(), 'e' | 'E') {
            is_float = true;
            self.position += 1;
            if matches!(self.peek(), '+' | '-') {
                self.position += 1;
            }
            while self.peek().is_ascii_digit() {
                self.position += 1;
            }
        }
        let text: String = self.chars[start..self.position].iter().collect();
        if is_float {
            match text.parse::<f64>() {
                Ok(value) => GodotValue::Float(value),
                Err(_) => {
                    self.warn(
                        "godot_parser_invalid_number",
                        format!("Invalid Godot number: {text}"),
                    );
                    GodotValue::Null
                }
            }
        } else {
            match text.parse::<i64>() {
                Ok(value) => GodotValue::Int(value),
                // Integer literal that overflows i64: keep it as a float rather than lose it.
                Err(_) => match text.parse::<f64>() {
                    Ok(value) => GodotValue::Float(value),
                    Err(_) => {
                        self.warn(
                            "godot_parser_invalid_number",
                            format!("Invalid Godot number: {text}"),
                        );
                        GodotValue::Null
                    }
                },
            }
        }
    }

    fn parse_identifier(&mut self) -> String {
        let start = self.position;
        while !self.done() && is_identifier_char(self.peek()) {
            self.position += 1;
        }
        // Consume a typed-collection suffix like `Array[NodePath]` so the whole
        // constructor name is captured (it later maps to Unsupported).
        if self.peek() == '[' {
            self.position += 1;
            while !self.done() && self.peek() != ']' {
                if !is_identifier_char(self.peek()) {
                    break;
                }
                self.position += 1;
            }
            if self.peek() == ']' {
                self.position += 1;
            }
        }
        self.chars[start..self.position].iter().collect()
    }

    fn parse_string_name(&mut self) -> GodotValue {
        self.position += 1;
        if self.peek() == '"' || self.peek() == '\'' {
            return GodotValue::StringName(self.parse_string());
        }
        self.warn(
            "godot_parser_string_name_expected_string",
            "Expected quoted string after '&' StringName marker.".to_string(),
        );
        GodotValue::StringName(String::new())
    }

    fn parse_call(&mut self, name: &str) -> GodotValue {
        self.position += 1;
        let mut args: Vec<GodotValue> = Vec::new();
        while !self.done() {
            self.skip_trivia();
            if self.peek() == ')' {
                self.position += 1;
                return call_value(name, args, self.ext_table);
            }
            args.push(self.parse_value());
            self.skip_trivia();
            if self.peek() == ',' {
                self.position += 1;
            }
        }
        self.warn(
            "godot_parser_unterminated_call",
            format!("Unterminated Godot call {name}(...)."),
        );
        call_value(name, args, self.ext_table)
    }
}

fn is_identifier_char(value: char) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, '_' | '.' | '/' | ':' | '-')
}

fn decode_escape(value: char) -> char {
    match value {
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        other => other,
    }
}

/// StringName / `&"k"` keys collapse to their inner string (as a plain `Str` key); every
/// other key is kept as its parsed value.
fn dictionary_key(key: GodotValue) -> GodotValue {
    match key {
        GodotValue::StringName(inner) => GodotValue::Str(inner),
        other => other,
    }
}

/// Map a `Name(args…)` constructor onto the matching [`GodotValue`]. Unknown
/// constructors, wrong arity, and non-numeric args where numbers are required all fall
/// back to [`GodotValue::Unsupported`].
fn call_value(
    name: &str,
    args: Vec<GodotValue>,
    ext_table: &HashMap<String, String>,
) -> GodotValue {
    match name {
        "Vector2" => or_unsupported(f32_array::<2>(&args).map(GodotValue::Vector2), name),
        "Vector2i" => or_unsupported(i32_array::<2>(&args).map(GodotValue::Vector2i), name),
        "Vector3" => or_unsupported(f32_array::<3>(&args).map(GodotValue::Vector3), name),
        "Vector3i" => or_unsupported(i32_array::<3>(&args).map(GodotValue::Vector3i), name),
        "Vector4" => or_unsupported(f32_array::<4>(&args).map(GodotValue::Vector4), name),
        "Vector4i" => or_unsupported(i32_array::<4>(&args).map(GodotValue::Vector4i), name),
        "Rect2" => or_unsupported(f32_array::<4>(&args).map(GodotValue::Rect2), name),
        "Rect2i" => or_unsupported(i32_array::<4>(&args).map(GodotValue::Rect2i), name),
        "Plane" => or_unsupported(f32_array::<4>(&args).map(GodotValue::Plane), name),
        "Quaternion" => or_unsupported(f32_array::<4>(&args).map(GodotValue::Quaternion), name),
        "AABB" => or_unsupported(f32_array::<6>(&args).map(GodotValue::Aabb), name),
        "Basis" => or_unsupported(f32_array::<9>(&args).map(GodotValue::Basis), name),
        "Transform2D" => or_unsupported(f32_array::<6>(&args).map(GodotValue::Transform2D), name),
        "Transform3D" => or_unsupported(f32_array::<12>(&args).map(GodotValue::Transform3D), name),
        "Projection" => or_unsupported(f32_array::<16>(&args).map(GodotValue::Projection), name),
        "Color" => or_unsupported(f32_array::<4>(&args).map(GodotValue::Color), name),
        "NodePath" => {
            let path = match args.first() {
                Some(GodotValue::Str(value)) => value.clone(),
                _ => String::new(),
            };
            GodotValue::NodePath(path)
        }
        "ExtResource" => {
            let id = resource_id(&args);
            match ext_table.get(&id) {
                Some(path) => GodotValue::ExtResource(path.clone()),
                None => GodotValue::Unsupported(format!("ExtResource:{id}")),
            }
        }
        "SubResource" => GodotValue::Unsupported(format!("SubResource:{}", resource_id(&args))),
        "PackedByteArray" => {
            or_unsupported(to_u8_vec(&args).map(GodotValue::PackedByteArray), name)
        }
        "PackedInt32Array" => {
            or_unsupported(to_i32_vec(&args).map(GodotValue::PackedInt32Array), name)
        }
        "PackedInt64Array" => {
            or_unsupported(to_i64_vec(&args).map(GodotValue::PackedInt64Array), name)
        }
        "PackedFloat32Array" => {
            or_unsupported(to_f32_vec(&args).map(GodotValue::PackedFloat32Array), name)
        }
        "PackedFloat64Array" => {
            or_unsupported(to_f64_vec(&args).map(GodotValue::PackedFloat64Array), name)
        }
        "PackedStringArray" => or_unsupported(
            to_string_vec(&args).map(GodotValue::PackedStringArray),
            name,
        ),
        "PackedVector2Array" => or_unsupported(
            packed_f32::<2>(&args).map(GodotValue::PackedVector2Array),
            name,
        ),
        "PackedVector3Array" => or_unsupported(
            packed_f32::<3>(&args).map(GodotValue::PackedVector3Array),
            name,
        ),
        "PackedColorArray" => or_unsupported(
            packed_f32::<4>(&args).map(GodotValue::PackedColorArray),
            name,
        ),
        other => GodotValue::Unsupported(other.to_string()),
    }
}

fn or_unsupported(value: Option<GodotValue>, name: &str) -> GodotValue {
    value.unwrap_or_else(|| GodotValue::Unsupported(name.to_string()))
}

fn resource_id(args: &[GodotValue]) -> String {
    match args.first() {
        Some(GodotValue::Str(value)) => value.clone(),
        Some(GodotValue::Int(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn as_f32(value: &GodotValue) -> Option<f32> {
    match value {
        GodotValue::Int(value) => Some(*value as f32),
        GodotValue::Float(value) => Some(*value as f32),
        _ => None,
    }
}

fn as_f64(value: &GodotValue) -> Option<f64> {
    match value {
        GodotValue::Int(value) => Some(*value as f64),
        GodotValue::Float(value) => Some(*value),
        _ => None,
    }
}

fn as_i32(value: &GodotValue) -> Option<i32> {
    match value {
        GodotValue::Int(value) => Some(*value as i32),
        GodotValue::Float(value) => Some(*value as i32),
        _ => None,
    }
}

fn as_i64(value: &GodotValue) -> Option<i64> {
    match value {
        GodotValue::Int(value) => Some(*value),
        GodotValue::Float(value) => Some(*value as i64),
        _ => None,
    }
}

fn f32_array<const N: usize>(args: &[GodotValue]) -> Option<[f32; N]> {
    if args.len() != N {
        return None;
    }
    let mut out = [0.0f32; N];
    for (slot, arg) in out.iter_mut().zip(args) {
        *slot = as_f32(arg)?;
    }
    Some(out)
}

fn i32_array<const N: usize>(args: &[GodotValue]) -> Option<[i32; N]> {
    if args.len() != N {
        return None;
    }
    let mut out = [0i32; N];
    for (slot, arg) in out.iter_mut().zip(args) {
        *slot = as_i32(arg)?;
    }
    Some(out)
}

fn packed_f32<const K: usize>(args: &[GodotValue]) -> Option<Vec<[f32; K]>> {
    if K == 0 || !args.len().is_multiple_of(K) {
        return None;
    }
    let mut out: Vec<[f32; K]> = Vec::with_capacity(args.len() / K);
    for chunk in args.chunks(K) {
        let mut group = [0.0f32; K];
        for (slot, arg) in group.iter_mut().zip(chunk) {
            *slot = as_f32(arg)?;
        }
        out.push(group);
    }
    Some(out)
}

fn to_u8_vec(args: &[GodotValue]) -> Option<Vec<u8>> {
    args.iter()
        .map(|arg| as_i64(arg).map(|value| value as u8))
        .collect()
}

fn to_i32_vec(args: &[GodotValue]) -> Option<Vec<i32>> {
    args.iter().map(as_i32).collect()
}

fn to_i64_vec(args: &[GodotValue]) -> Option<Vec<i64>> {
    args.iter().map(as_i64).collect()
}

fn to_f32_vec(args: &[GodotValue]) -> Option<Vec<f32>> {
    args.iter().map(as_f32).collect()
}

fn to_f64_vec(args: &[GodotValue]) -> Option<Vec<f64>> {
    args.iter().map(as_f64).collect()
}

fn to_string_vec(args: &[GodotValue]) -> Option<Vec<String>> {
    args.iter()
        .map(|arg| match arg {
            GodotValue::Str(value) => Some(value.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse_value(source: &str) -> GodotValue {
        let mut diagnostics = Vec::new();
        parse_godot_value(source, &mut diagnostics, None, &HashMap::new())
    }

    fn node_props(node: &SceneNode) -> HashMap<String, GodotValue> {
        node.properties.iter().cloned().collect()
    }

    fn sample_scene(relative: &str) -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/sample_project")
            .join(relative);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    }

    // -- Value grammar ------------------------------------------------------

    #[test]
    fn preserves_int_versus_float() {
        assert_eq!(parse_value("1"), GodotValue::Int(1));
        assert_eq!(parse_value("-42"), GodotValue::Int(-42));
        assert_eq!(parse_value("1.0"), GodotValue::Float(1.0));
        assert_eq!(parse_value("1e3"), GodotValue::Float(1000.0));
        assert_eq!(parse_value(".5"), GodotValue::Float(0.5));
    }

    #[test]
    fn parses_scalars_and_bare_identifiers() {
        assert_eq!(parse_value("true"), GodotValue::Bool(true));
        assert_eq!(parse_value("false"), GodotValue::Bool(false));
        assert_eq!(parse_value("null"), GodotValue::Null);
        assert_eq!(parse_value("nil"), GodotValue::Null);
        assert_eq!(
            parse_value("\"hello\""),
            GodotValue::Str("hello".to_string())
        );
        assert_eq!(
            parse_value("bare_ident"),
            GodotValue::Str("bare_ident".to_string())
        );
    }

    #[test]
    fn parses_string_escapes() {
        assert_eq!(
            parse_value("\"a\\nb\\tc\""),
            GodotValue::Str("a\nb\tc".to_string())
        );
    }

    #[test]
    fn parses_math_constructors() {
        assert_eq!(
            parse_value("Vector2(1, 2)"),
            GodotValue::Vector2([1.0, 2.0])
        );
        assert_eq!(parse_value("Vector2i(3, 4)"), GodotValue::Vector2i([3, 4]));
        assert_eq!(
            parse_value("Color(1, 0, 0, 1)"),
            GodotValue::Color([1.0, 0.0, 0.0, 1.0])
        );
        assert_eq!(
            parse_value("Rect2(1, 2, 30, 40)"),
            GodotValue::Rect2([1.0, 2.0, 30.0, 40.0])
        );
        assert_eq!(
            parse_value("Transform2D(1, 0, 0, 1, 0, 0)"),
            GodotValue::Transform2D([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
        );
    }

    #[test]
    fn wrong_arity_constructor_is_unsupported() {
        assert_eq!(
            parse_value("Vector2(1)"),
            GodotValue::Unsupported("Vector2".to_string())
        );
        assert_eq!(
            parse_value("Color(1, 2, 3)"),
            GodotValue::Unsupported("Color".to_string())
        );
    }

    #[test]
    fn parses_node_path_and_string_name() {
        assert_eq!(
            parse_value("NodePath(\"../X\")"),
            GodotValue::NodePath("../X".to_string())
        );
        assert_eq!(
            parse_value("&\"default\""),
            GodotValue::StringName("default".to_string())
        );
    }

    #[test]
    fn unknown_constructor_and_sub_resource_are_unsupported() {
        assert_eq!(
            parse_value("SubResource(\"A\")"),
            GodotValue::Unsupported("SubResource:A".to_string())
        );
        assert_eq!(
            parse_value("Array[NodePath]([])"),
            GodotValue::Unsupported("Array[NodePath]".to_string())
        );
        assert_eq!(
            parse_value("SomeObject(1, 2)"),
            GodotValue::Unsupported("SomeObject".to_string())
        );
    }

    #[test]
    fn ext_resource_resolves_against_table_else_unsupported() {
        let mut table = HashMap::new();
        table.insert("1".to_string(), "res://icon.png".to_string());
        let mut diagnostics = Vec::new();
        assert_eq!(
            parse_godot_value("ExtResource(\"1\")", &mut diagnostics, None, &table),
            GodotValue::ExtResource("res://icon.png".to_string())
        );
        assert_eq!(
            parse_godot_value("ExtResource(\"9\")", &mut diagnostics, None, &table),
            GodotValue::Unsupported("ExtResource:9".to_string())
        );
    }

    #[test]
    fn parses_arrays_with_mixed_and_null_elements() {
        assert_eq!(
            parse_value("[NodePath(\"shiv\"), null, null]"),
            GodotValue::Array(vec![
                GodotValue::NodePath("shiv".to_string()),
                GodotValue::Null,
                GodotValue::Null,
            ])
        );
    }

    #[test]
    fn parses_packed_arrays() {
        assert_eq!(
            parse_value("PackedVector2Array(20, -135, 600, -150)"),
            GodotValue::PackedVector2Array(vec![[20.0, -135.0], [600.0, -150.0]])
        );
        assert_eq!(
            parse_value("PackedColorArray(1, 1, 1, 0, 1, 1, 1, 1)"),
            GodotValue::PackedColorArray(vec![[1.0, 1.0, 1.0, 0.0], [1.0, 1.0, 1.0, 1.0]])
        );
        assert_eq!(
            parse_value("PackedInt32Array(1, 2, 3)"),
            GodotValue::PackedInt32Array(vec![1, 2, 3])
        );
        assert_eq!(
            parse_value("PackedStringArray(\"a\", \"b\")"),
            GodotValue::PackedStringArray(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn parses_dictionary_with_string_name_keys() {
        let value = parse_value("{&\"\": null, &\"idle\": true, \"n\": 1}");
        assert_eq!(
            value,
            GodotValue::Dictionary(vec![
                (GodotValue::Str(String::new()), GodotValue::Null),
                (GodotValue::Str("idle".to_string()), GodotValue::Bool(true)),
                (GodotValue::Str("n".to_string()), GodotValue::Int(1)),
            ])
        );
    }

    // -- Sections & node tree ----------------------------------------------

    #[test]
    fn parses_sections_node_tree_and_ext_resolution() {
        let scene = parse_scene(
            "[gd_scene load_steps=1 format=3]\n\
             [ext_resource type=\"Texture2D\" path=\"res://ui/panel.png\" id=\"1\"]\n\
             [node name=\"Root\" type=\"Control\"]\n\
             anchor_right = 1.0\n\
             [node name=\"Panel\" type=\"ColorRect\" parent=\".\"]\n\
             color = Color(1, 0, 0, 1)\n\
             texture = ExtResource(\"1\")\n",
        );
        assert!(!scene.has_errors());
        assert_eq!(scene.ext_resources.len(), 1);
        assert_eq!(
            scene.ext_resources[0].path.as_deref(),
            Some("res://ui/panel.png")
        );
        assert_eq!(scene.nodes.len(), 2);

        let root = scene.root().unwrap();
        assert_eq!(root.name, "Root");
        assert_eq!(root.type_name.as_deref(), Some("Control"));
        assert!(root.parent.is_none());

        let panel = &scene.nodes[1];
        assert_eq!(panel.parent.as_deref(), Some("."));
        let props = node_props(panel);
        assert_eq!(props["color"], GodotValue::Color([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(
            props["texture"],
            GodotValue::ExtResource("res://ui/panel.png".to_string())
        );
    }

    #[test]
    fn parses_groups_index_connection_and_editable() {
        let scene = parse_scene(
            "[gd_scene format=3]\n\
             [node name=\"Root\" type=\"Control\"]\n\
             [node name=\"Panel\" type=\"ColorRect\" parent=\".\" index=\"2\" groups=[\"ui\", \"debug\"]]\n\
             [connection signal=\"pressed\" from=\"Panel\" to=\".\" method=\"_on_pressed\"]\n\
             [editable path=\"Panel\"]\n",
        );
        assert!(!scene.has_errors());
        let panel = &scene.nodes[1];
        assert_eq!(panel.index, Some(2));
        assert_eq!(panel.groups, vec!["ui".to_string(), "debug".to_string()]);
        assert_eq!(scene.connections.len(), 1);
        assert_eq!(scene.connections[0].signal, "pressed");
        assert_eq!(scene.connections[0].method, "_on_pressed");
        assert_eq!(scene.editable, vec!["Panel".to_string()]);
    }

    #[test]
    fn resolves_instance_and_flags_placeholders() {
        let scene = parse_scene(
            "[gd_scene format=3]\n\
             [ext_resource type=\"PackedScene\" path=\"res://child.tscn\" id=\"2\"]\n\
             [node name=\"Root\" type=\"Control\"]\n\
             [node name=\"Inst\" parent=\".\" instance=ExtResource(\"2\")]\n\
             [node name=\"Ghost\" parent=\".\" instance=SubResource(\"S\")]\n\
             [node name=\"Holder\" parent=\".\" instance_placeholder=\"res://p.tscn\"]\n",
        );
        assert!(!scene.has_errors());
        let inst = &scene.nodes[1];
        assert_eq!(inst.instance_scene.as_deref(), Some("res://child.tscn"));
        assert!(!inst.instance_unresolved);
        assert!(inst.is_instance());

        let ghost = &scene.nodes[2];
        assert!(ghost.instance_scene.is_none());
        assert!(ghost.instance_unresolved);

        let holder = &scene.nodes[3];
        assert!(holder.instance_unresolved);
    }

    #[test]
    fn last_write_wins_but_keeps_order_on_duplicate_key() {
        let scene = parse_scene(
            "[gd_scene format=3]\n\
             [node name=\"Root\" type=\"Node\"]\n\
             a = 1\n\
             b = 2\n\
             a = 3\n",
        );
        let root = scene.root().unwrap();
        let keys: Vec<&str> = root
            .properties
            .iter()
            .map(|(key, _)| key.as_str())
            .collect();
        assert_eq!(keys, vec!["a", "b"]);
        assert_eq!(root.property("a"), Some(&GodotValue::Int(3)));
    }

    // -- Multi-line values --------------------------------------------------

    #[test]
    fn preserves_multiline_quoted_strings_and_editable() {
        let scene = parse_scene(
            "[gd_scene load_steps=1 format=3]\n\
             [node name=\"Root\" type=\"Control\"]\n\
             [node name=\"Text\" type=\"RichTextLabel\" parent=\".\"]\n\
             text = \"[b]Line one[/b]\nLine two\"\n\
             [editable path=\"Text\"]\n",
        );
        assert!(!scene.has_errors());
        let text = &scene.nodes[1];
        assert_eq!(
            text.property("text"),
            Some(&GodotValue::Str("[b]Line one[/b]\nLine two".to_string()))
        );
        assert_eq!(scene.editable, vec!["Text".to_string()]);
    }

    #[test]
    fn parses_multiline_sub_resource_dictionaries() {
        let scene = parse_scene(
            "[gd_scene load_steps=2 format=3]\n\
             [ext_resource type=\"Texture2D\" path=\"res://frame.png\" id=\"1\"]\n\
             [sub_resource type=\"SpriteFrames\" id=\"SpriteFrames_demo\"]\n\
             animations = [{\n\
             \"frames\": [{\n\
             \"duration\": 1.0,\n\
             \"texture\": ExtResource(\"1\")\n\
             }],\n\
             \"loop\": true,\n\
             \"name\": &\"default\",\n\
             \"speed\": 10.0\n\
             }]\n\
             [node name=\"Root\" type=\"Control\"]\n",
        );
        assert!(!scene.has_errors());
        assert_eq!(scene.sub_resources.len(), 1);
        let animations = scene.sub_resources[0]
            .properties
            .iter()
            .find(|(key, _)| key == "animations")
            .map(|(_, value)| value)
            .unwrap();
        let expected = GodotValue::Array(vec![GodotValue::Dictionary(vec![
            (
                GodotValue::Str("frames".to_string()),
                GodotValue::Array(vec![GodotValue::Dictionary(vec![
                    (
                        GodotValue::Str("duration".to_string()),
                        GodotValue::Float(1.0),
                    ),
                    (
                        GodotValue::Str("texture".to_string()),
                        GodotValue::ExtResource("res://frame.png".to_string()),
                    ),
                ])]),
            ),
            (GodotValue::Str("loop".to_string()), GodotValue::Bool(true)),
            (
                GodotValue::Str("name".to_string()),
                GodotValue::StringName("default".to_string()),
            ),
            (
                GodotValue::Str("speed".to_string()),
                GodotValue::Float(10.0),
            ),
        ])]);
        assert_eq!(animations, &expected);
    }

    #[test]
    fn quote_aware_line_comment_stripping() {
        let scene = parse_scene(
            "[gd_scene format=3]\n\
             [node name=\"Root\" type=\"Node\"]\n\
             a = \"has ; semicolon\" ; trailing comment\n",
        );
        let root = scene.root().unwrap();
        assert_eq!(
            root.property("a"),
            Some(&GodotValue::Str("has ; semicolon".to_string()))
        );
    }

    // -- Checked-in sample scenes ------------------------------------------

    #[test]
    fn parses_sample_main_menu() {
        let scene = parse_scene(&sample_scene("ui/main_menu.tscn"));
        assert!(
            !scene.has_errors(),
            "unexpected error diagnostics: {:?}",
            scene.diagnostics
        );

        let root = scene.root().unwrap();
        assert_eq!(root.name, "MainMenu");
        assert_eq!(root.type_name.as_deref(), Some("Control"));
        assert_eq!(
            root.property("script"),
            Some(&GodotValue::ExtResource(
                "res://ui/main_menu.gd".to_string()
            ))
        );

        // Direct children of the root.
        let root_children: Vec<&str> = scene
            .nodes
            .iter()
            .filter(|node| node.parent.as_deref() == Some("."))
            .map(|node| node.name.as_str())
            .collect();
        assert!(root_children.contains(&"Canvas"));
        assert!(root_children.contains(&"PauseMenu"));

        // The instanced PauseMenu resolves to its sub-scene path.
        let pause = scene
            .nodes
            .iter()
            .find(|node| node.name == "PauseMenu")
            .unwrap();
        assert_eq!(
            pause.instance_scene.as_deref(),
            Some("res://ui/pause_menu.tscn")
        );
        assert!(pause.is_instance());

        // A Vector2 property round-trips as f32.
        let start = scene
            .nodes
            .iter()
            .find(|node| node.name == "StartButton")
            .unwrap();
        assert_eq!(
            start.property("custom_minimum_size"),
            Some(&GodotValue::Vector2([280.0, 80.0]))
        );
    }

    #[test]
    fn parses_sample_scenes_without_errors() {
        for relative in [
            "ui/main_menu.tscn",
            "ui/settings_menu.tscn",
            "ui/pause_menu.tscn",
            "ui/fixtures/physics_inspect_demo.tscn",
        ] {
            let scene = parse_scene(&sample_scene(relative));
            assert!(
                !scene.has_errors(),
                "{relative} produced error diagnostics: {:?}",
                scene.diagnostics
            );
            assert!(scene.root().is_some(), "{relative} has no root node");
        }
    }

    #[test]
    fn sub_resource_reference_is_unsupported() {
        let scene = parse_scene(&sample_scene("ui/fixtures/physics_inspect_demo.tscn"));
        let shape_node = scene
            .nodes
            .iter()
            .find(|node| node.property("shape").is_some())
            .unwrap();
        match shape_node.property("shape") {
            Some(GodotValue::Unsupported(reason)) => assert!(reason.starts_with("SubResource:")),
            other => panic!("expected Unsupported SubResource, got {other:?}"),
        }
    }

    /// Regression: `parse_value` must consume a character even when no value
    /// grammar rule matches it. Before that guarantee, container loops
    /// (array / dictionary / call arguments) retried `parse_value` at the same
    /// position forever, growing their element vectors unboundedly — the
    /// 40-byte input below allocated gigabytes before aborting. Found by the
    /// fuzz harness in tests/fuzz_corpus.rs (audio_demo.tscn mutant,
    /// seed 0xB89E4B17DBA1E58A).
    #[test]
    fn unparseable_char_inside_call_array_terminates() {
        let scene = "[gd_scene format=3]\n[node name=\"R\" type=\"Node\"]\na = Foo([)\n";
        let state = parse_scene(scene);
        assert!(!state.diagnostics.is_empty());
    }

    #[test]
    fn unparseable_chars_inside_plain_array_terminate() {
        let scene =
            "[gd_scene format=3]\n[node name=\"R\" type=\"Node\"]\na = [$=\\)]\nb = {$: $}\n";
        let state = parse_scene(scene);
        assert!(!state.diagnostics.is_empty());
    }
}
