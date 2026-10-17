use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::PathBuf;

pub const COMMAND: &str = "qa_state.validate";

#[derive(Debug, Clone)]
pub struct QaStateValidateRequest {
    pub schema: PathBuf,
    pub input: Option<PathBuf>,
    pub trace: Option<PathBuf>,
    pub state_path: Option<String>,
    pub target: Option<String>,
    pub method: Option<String>,
    pub provider: Option<String>,
}

#[derive(Debug)]
pub struct SchemaValidator {
    schema: Value,
}

#[derive(Debug, Clone)]
struct SelectedValue {
    json_path: String,
    value: Value,
}

#[derive(Debug, Clone)]
struct SchemaValidationError {
    json_path: String,
    schema_path: String,
    expected: String,
    actual: String,
    actual_value_preview: String,
}

#[derive(Debug, Default)]
struct ValidationContext {
    errors: Vec<SchemaValidationError>,
    checked_paths: usize,
}

impl SchemaValidationError {
    fn to_json(&self) -> Value {
        json!({
            "json_path": self.json_path,
            "schema_path": self.schema_path,
            "expected": self.expected,
            "actual": self.actual,
            "actual_value_preview": self.actual_value_preview,
        })
    }
}

impl SchemaValidator {
    pub fn new(schema: Value) -> Result<Self, FailureDetail> {
        let mut unsupported = Vec::new();
        scan_supported_keywords(&schema, "$", &mut unsupported);
        if let Some((schema_path, keyword)) = unsupported.into_iter().next() {
            return Err(FailureDetail {
                kind: "qa_state_schema_unsupported_keyword".to_string(),
                command: Some(COMMAND.to_string()),
                message: Some(format!(
                    "schema keyword '{keyword}' is not supported by godot-qa's local schema subset"
                )),
                expected: Some(json!(supported_keywords())),
                actual: Some(json!(keyword)),
                path: Some(schema_path),
                ..FailureDetail::default()
            });
        }
        Ok(Self { schema })
    }

    fn validate(&self, selected: &[SelectedValue]) -> (usize, Vec<SchemaValidationError>) {
        let mut context = ValidationContext::default();
        for value in selected {
            validate_schema_value(
                &value.value,
                &self.schema,
                &value.json_path,
                "$",
                &mut context,
            );
        }
        (context.checked_paths, context.errors)
    }
}

pub fn qa_state_validate_result(request: QaStateValidateRequest) -> CommandResult {
    if request.target.is_some() || request.method.is_some() || request.provider.is_some() {
        return live_not_implemented_result(&request);
    }
    qa_state_validate_selected_result(request, None)
}

pub fn qa_state_validate_value_result(
    request: QaStateValidateRequest,
    state: Value,
) -> CommandResult {
    qa_state_validate_selected_result(
        request,
        Some(vec![SelectedValue {
            json_path: "$".to_string(),
            value: state,
        }]),
    )
}

fn qa_state_validate_selected_result(
    request: QaStateValidateRequest,
    selected_override: Option<Vec<SelectedValue>>,
) -> CommandResult {
    let schema = match read_json_file(&request.schema) {
        Ok(schema) => schema,
        Err(failure) => return failure_result(failure, "schema validation could not start", 1),
    };
    let validator = match SchemaValidator::new(schema) {
        Ok(validator) => validator,
        Err(failure) => return failure_result(failure, "schema validation could not start", 1),
    };

    let selected = match selected_override {
        Some(selected) => selected,
        None => match selected_values_from_request(&request) {
            Ok(selected) => selected,
            Err(failure) => return failure_result(failure, "schema validation could not start", 1),
        },
    };

    let (checked_paths, errors) = validator.validate(&selected);
    let warnings: Vec<Value> = Vec::new();
    let mut data = Map::from_iter([
        ("schema".to_string(), json!(request.schema)),
        (
            "summary".to_string(),
            json!({
                "checked_paths": checked_paths,
                "selected_values": selected.len(),
                "warnings": warnings,
            }),
        ),
    ]);
    if let Some(input) = &request.input {
        data.insert("input".to_string(), json!(input));
    }
    if let Some(trace) = &request.trace {
        data.insert("trace".to_string(), json!(trace));
    }
    if let Some(state_path) = &request.state_path {
        data.insert("state_path".to_string(), json!(state_path));
    }
    if let Some(target) = &request.target {
        data.insert("target".to_string(), json!(target));
    }
    if let Some(method) = &request.method {
        data.insert("method".to_string(), json!(method));
    }

    if errors.is_empty() {
        return CommandResult {
            ok: true,
            command: COMMAND.to_string(),
            message: format!("validated {} QA state value(s)", selected.len()),
            data,
            failures: Vec::new(),
            exit_code: 0,
        };
    }

    let error_values: Vec<Value> = errors.iter().map(SchemaValidationError::to_json).collect();
    data.insert("errors".to_string(), json!(error_values));
    CommandResult {
        ok: false,
        command: COMMAND.to_string(),
        message: "QA state schema validation failed".to_string(),
        data,
        failures: vec![FailureDetail {
            kind: "qa_state_schema_validation_failed".to_string(),
            command: Some(COMMAND.to_string()),
            message: Some(
                "one or more selected QA state values did not match the schema".to_string(),
            ),
            expected: Some(json!("selected values match schema")),
            actual: Some(json!("schema validation errors")),
            details: Map::from_iter([("errors".to_string(), json!(error_values))]),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

pub fn select_state_path_values(
    value: &Value,
    path: &str,
) -> Result<Vec<(String, Value)>, FailureDetail> {
    let segments = parse_state_path(path)?;
    let mut selected = vec![SelectedValue {
        json_path: "$".to_string(),
        value: value.clone(),
    }];
    for segment in segments {
        let mut next = Vec::new();
        for item in selected {
            match segment {
                PathSegment::Field(ref field) => {
                    if let Some(child) = item.value.get(field) {
                        next.push(SelectedValue {
                            json_path: format!("{}.{}", item.json_path, field),
                            value: child.clone(),
                        });
                    }
                }
                PathSegment::FieldWildcard(ref field) => {
                    if let Some(array) = item.value.get(field).and_then(Value::as_array) {
                        for (index, child) in array.iter().enumerate() {
                            next.push(SelectedValue {
                                json_path: format!("{}.{}[{index}]", item.json_path, field),
                                value: child.clone(),
                            });
                        }
                    }
                }
                PathSegment::FieldIndex(ref field, expected_index) => {
                    if let Some(child) = item
                        .value
                        .get(field)
                        .and_then(Value::as_array)
                        .and_then(|array| array.get(expected_index))
                    {
                        next.push(SelectedValue {
                            json_path: format!("{}.{}[{expected_index}]", item.json_path, field),
                            value: child.clone(),
                        });
                    }
                }
            }
        }
        selected = next;
    }
    Ok(selected
        .into_iter()
        .map(|selected| (selected.json_path, selected.value))
        .collect())
}

fn selected_values_from_request(
    request: &QaStateValidateRequest,
) -> Result<Vec<SelectedValue>, FailureDetail> {
    match (&request.input, &request.trace) {
        (Some(input), None) => Ok(vec![SelectedValue {
            json_path: "$".to_string(),
            value: read_json_file(input)?,
        }]),
        (None, Some(trace)) => {
            let state_path = request.state_path.as_deref().ok_or_else(|| FailureDetail {
                kind: "invalid_params".to_string(),
                command: Some(COMMAND.to_string()),
                message: Some("qa-state validate --trace requires --state-path".to_string()),
                expected: Some(json!("--state-path such as samples[*].state")),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            })?;
            let trace_value = read_json_file(trace)?;
            let selected = select_state_path_values(&trace_value, state_path)?;
            if selected.is_empty() {
                return Err(FailureDetail {
                    kind: "qa_state_path_not_found".to_string(),
                    command: Some(COMMAND.to_string()),
                    message: Some("state path did not select any values from trace".to_string()),
                    expected: Some(json!("one or more selected values")),
                    actual: Some(json!("empty selection")),
                    path: Some(state_path.to_string()),
                    ..FailureDetail::default()
                });
            }
            Ok(selected
                .into_iter()
                .map(|(json_path, value)| SelectedValue { json_path, value })
                .collect())
        }
        _ => Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(COMMAND.to_string()),
            message: Some("qa-state validate requires exactly one source mode: --input, --trace, or --target/--method".to_string()),
            expected: Some(json!("exactly one QA state source")),
            actual: Some(json!("invalid source mode")),
            ..FailureDetail::default()
        }),
    }
}

fn validate_schema_value(
    value: &Value,
    schema: &Value,
    json_path: &str,
    schema_path: &str,
    context: &mut ValidationContext,
) {
    let Some(schema_object) = schema.as_object() else {
        return;
    };

    if let Some(expected_type) = schema_object.get("type") {
        context.checked_paths += 1;
        if !matches_type(value, expected_type) {
            context.errors.push(SchemaValidationError {
                json_path: json_path.to_string(),
                schema_path: format!("{schema_path}.type"),
                expected: expected_type_label(expected_type),
                actual: actual_type(value).to_string(),
                actual_value_preview: value_preview(value),
            });
            return;
        }
    }

    if let Some(const_value) = schema_object.get("const") {
        context.checked_paths += 1;
        if value != const_value {
            context.errors.push(SchemaValidationError {
                json_path: json_path.to_string(),
                schema_path: format!("{schema_path}.const"),
                expected: format!("const {}", value_preview(const_value)),
                actual: actual_type(value).to_string(),
                actual_value_preview: value_preview(value),
            });
        }
    }

    if let Some(enum_values) = schema_object.get("enum").and_then(Value::as_array) {
        context.checked_paths += 1;
        if !enum_values.iter().any(|allowed| allowed == value) {
            context.errors.push(SchemaValidationError {
                json_path: json_path.to_string(),
                schema_path: format!("{schema_path}.enum"),
                expected: "one of enum values".to_string(),
                actual: actual_type(value).to_string(),
                actual_value_preview: value_preview(value),
            });
        }
    }

    validate_number_bounds(value, schema_object, json_path, schema_path, context);
    validate_string_bounds(value, schema_object, json_path, schema_path, context);
    validate_array(value, schema_object, json_path, schema_path, context);
    validate_object(value, schema_object, json_path, schema_path, context);
}

fn validate_number_bounds(
    value: &Value,
    schema: &Map<String, Value>,
    json_path: &str,
    schema_path: &str,
    context: &mut ValidationContext,
) {
    let Some(number) = value.as_f64() else {
        return;
    };
    if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64) {
        context.checked_paths += 1;
        if number < minimum {
            context.errors.push(SchemaValidationError {
                json_path: json_path.to_string(),
                schema_path: format!("{schema_path}.minimum"),
                expected: format!("number >= {minimum}"),
                actual: format_number(number),
                actual_value_preview: value_preview(value),
            });
        }
    }
    if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64) {
        context.checked_paths += 1;
        if number > maximum {
            context.errors.push(SchemaValidationError {
                json_path: json_path.to_string(),
                schema_path: format!("{schema_path}.maximum"),
                expected: format!("number <= {maximum}"),
                actual: format_number(number),
                actual_value_preview: value_preview(value),
            });
        }
    }
}

fn validate_string_bounds(
    value: &Value,
    schema: &Map<String, Value>,
    json_path: &str,
    schema_path: &str,
    context: &mut ValidationContext,
) {
    let Some(text) = value.as_str() else {
        return;
    };
    if let Some(min_length) = schema.get("minLength").and_then(Value::as_u64) {
        context.checked_paths += 1;
        let actual = text.chars().count() as u64;
        if actual < min_length {
            context.errors.push(SchemaValidationError {
                json_path: json_path.to_string(),
                schema_path: format!("{schema_path}.minLength"),
                expected: format!("string length >= {min_length}"),
                actual: format!("string length {actual}"),
                actual_value_preview: value_preview(value),
            });
        }
    }
    if let Some(max_length) = schema.get("maxLength").and_then(Value::as_u64) {
        context.checked_paths += 1;
        let actual = text.chars().count() as u64;
        if actual > max_length {
            context.errors.push(SchemaValidationError {
                json_path: json_path.to_string(),
                schema_path: format!("{schema_path}.maxLength"),
                expected: format!("string length <= {max_length}"),
                actual: format!("string length {actual}"),
                actual_value_preview: value_preview(value),
            });
        }
    }
}

fn validate_array(
    value: &Value,
    schema: &Map<String, Value>,
    json_path: &str,
    schema_path: &str,
    context: &mut ValidationContext,
) {
    let Some(array) = value.as_array() else {
        return;
    };
    if let Some(min_items) = schema.get("minItems").and_then(Value::as_u64) {
        context.checked_paths += 1;
        if (array.len() as u64) < min_items {
            context.errors.push(SchemaValidationError {
                json_path: json_path.to_string(),
                schema_path: format!("{schema_path}.minItems"),
                expected: format!("array length >= {min_items}"),
                actual: format!("array length {}", array.len()),
                actual_value_preview: value_preview(value),
            });
        }
    }
    if let Some(max_items) = schema.get("maxItems").and_then(Value::as_u64) {
        context.checked_paths += 1;
        if (array.len() as u64) > max_items {
            context.errors.push(SchemaValidationError {
                json_path: json_path.to_string(),
                schema_path: format!("{schema_path}.maxItems"),
                expected: format!("array length <= {max_items}"),
                actual: format!("array length {}", array.len()),
                actual_value_preview: value_preview(value),
            });
        }
    }
    if let Some(item_schema) = schema.get("items") {
        for (index, child) in array.iter().enumerate() {
            validate_schema_value(
                child,
                item_schema,
                &format!("{json_path}[{index}]"),
                &format!("{schema_path}.items"),
                context,
            );
        }
    }
}

fn validate_object(
    value: &Value,
    schema: &Map<String, Value>,
    json_path: &str,
    schema_path: &str,
    context: &mut ValidationContext,
) {
    let Some(object) = value.as_object() else {
        return;
    };
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for required_field in required.iter().filter_map(Value::as_str) {
            context.checked_paths += 1;
            if !object.contains_key(required_field) {
                context.errors.push(SchemaValidationError {
                    json_path: format!("{json_path}.{required_field}"),
                    schema_path: format!("{schema_path}.required"),
                    expected: "required property present".to_string(),
                    actual: "missing".to_string(),
                    actual_value_preview: "missing".to_string(),
                });
            }
        }
    }

    let properties = schema.get("properties").and_then(Value::as_object);
    if let Some(properties) = properties {
        for (field, child_schema) in properties {
            if let Some(child_value) = object.get(field) {
                validate_schema_value(
                    child_value,
                    child_schema,
                    &format!("{json_path}.{field}"),
                    &format!("{schema_path}.properties.{field}"),
                    context,
                );
            }
        }
    }

    if let Some(additional) = schema.get("additionalProperties") {
        match additional {
            Value::Bool(false) => {
                if let Some(properties) = properties {
                    for field in object
                        .keys()
                        .filter(|field| !properties.contains_key(*field))
                    {
                        context.checked_paths += 1;
                        context.errors.push(SchemaValidationError {
                            json_path: format!("{json_path}.{field}"),
                            schema_path: format!("{schema_path}.additionalProperties"),
                            expected: "no additional properties".to_string(),
                            actual: "additional property".to_string(),
                            actual_value_preview: value_preview(
                                object.get(field).unwrap_or(&Value::Null),
                            ),
                        });
                    }
                }
            }
            Value::Object(_) => {
                if let Some(properties) = properties {
                    for (field, child_value) in object
                        .iter()
                        .filter(|(field, _)| !properties.contains_key(*field))
                    {
                        validate_schema_value(
                            child_value,
                            additional,
                            &format!("{json_path}.{field}"),
                            &format!("{schema_path}.additionalProperties"),
                            context,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

fn scan_supported_keywords(
    schema: &Value,
    schema_path: &str,
    unsupported: &mut Vec<(String, String)>,
) {
    let Some(object) = schema.as_object() else {
        return;
    };
    for (key, value) in object {
        if is_supported_keyword(key) || is_annotation_keyword(key) {
            match key.as_str() {
                "properties" => {
                    if let Some(properties) = value.as_object() {
                        for (field, child_schema) in properties {
                            scan_supported_keywords(
                                child_schema,
                                &format!("{schema_path}.properties.{field}"),
                                unsupported,
                            );
                        }
                    }
                }
                "items" | "additionalProperties" if value.is_object() => {
                    scan_supported_keywords(value, &format!("{schema_path}.{key}"), unsupported);
                }
                _ => {}
            }
        } else {
            unsupported.push((format!("{schema_path}.{key}"), key.clone()));
        }
    }
}

fn is_supported_keyword(key: &str) -> bool {
    supported_keywords().contains(&key)
}

fn is_annotation_keyword(key: &str) -> bool {
    matches!(
        key,
        "$schema" | "$id" | "title" | "description" | "default" | "examples"
    )
}

fn supported_keywords() -> Vec<&'static str> {
    vec![
        "type",
        "properties",
        "required",
        "items",
        "enum",
        "const",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
        "minItems",
        "maxItems",
        "additionalProperties",
    ]
}

fn read_json_file(path: &PathBuf) -> Result<Value, FailureDetail> {
    let content = fs::read_to_string(path).map_err(|error| FailureDetail {
        kind: "qa_state_read_failed".to_string(),
        command: Some(COMMAND.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("readable JSON file")),
        actual: Some(json!("read failed")),
        ..FailureDetail::default()
    })?;
    serde_json::from_str(&content).map_err(|error| FailureDetail {
        kind: "qa_state_invalid_json".to_string(),
        command: Some(COMMAND.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("valid JSON")),
        actual: Some(json!("parse failed")),
        ..FailureDetail::default()
    })
}

#[derive(Debug, Clone)]
enum PathSegment {
    Field(String),
    FieldWildcard(String),
    FieldIndex(String, usize),
}

fn parse_state_path(path: &str) -> Result<Vec<PathSegment>, FailureDetail> {
    if path.trim().is_empty() || path.starts_with('$') {
        return Err(invalid_state_path(path));
    }
    path.split('.')
        .map(|part| {
            if part.is_empty() {
                return Err(invalid_state_path(path));
            }
            if let Some(field) = part.strip_suffix("[*]") {
                if field.is_empty() {
                    return Err(invalid_state_path(path));
                }
                return Ok(PathSegment::FieldWildcard(field.to_string()));
            }
            if let Some(open) = part.find('[') {
                if !part.ends_with(']') || open == 0 {
                    return Err(invalid_state_path(path));
                }
                let field = &part[..open];
                let index = part[open + 1..part.len() - 1]
                    .parse::<usize>()
                    .map_err(|_| invalid_state_path(path))?;
                return Ok(PathSegment::FieldIndex(field.to_string(), index));
            }
            Ok(PathSegment::Field(part.to_string()))
        })
        .collect()
}

fn invalid_state_path(path: &str) -> FailureDetail {
    FailureDetail {
        kind: "qa_state_invalid_state_path".to_string(),
        command: Some(COMMAND.to_string()),
        message: Some(
            "state path supports dot fields plus [*] or [index] array selectors".to_string(),
        ),
        expected: Some(json!("path such as samples[*].state")),
        actual: Some(json!(path)),
        ..FailureDetail::default()
    }
}

fn matches_type(value: &Value, expected_type: &Value) -> bool {
    match expected_type {
        Value::String(expected) => matches_single_type(value, expected),
        Value::Array(types) => types
            .iter()
            .filter_map(Value::as_str)
            .any(|expected| matches_single_type(value, expected)),
        _ => true,
    }
}

fn matches_single_type(value: &Value, expected: &str) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn expected_type_label(expected_type: &Value) -> String {
    match expected_type {
        Value::String(expected) => expected.clone(),
        Value::Array(_) => value_preview(expected_type),
        _ => "valid JSON Schema type".to_string(),
    }
}

fn actual_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn value_preview(value: &Value) -> String {
    let mut preview = match value {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "<unprintable>".to_string()),
    };
    if preview.chars().count() > 120 {
        preview = preview.chars().take(117).collect::<String>();
        preview.push_str("...");
    }
    preview
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn failure_result(failure: FailureDetail, message: &str, exit_code: i32) -> CommandResult {
    CommandResult {
        ok: false,
        command: COMMAND.to_string(),
        message: message.to_string(),
        data: Map::new(),
        failures: vec![failure],
        exit_code,
    }
}

fn live_not_implemented_result(request: &QaStateValidateRequest) -> CommandResult {
    CommandResult {
        ok: false,
        command: COMMAND.to_string(),
        message: "live QA state validation is not implemented in this bounded task".to_string(),
        data: Map::from_iter([
            ("schema".to_string(), json!(request.schema)),
            ("target".to_string(), json!(request.target)),
            ("method".to_string(), json!(request.method)),
        ]),
        failures: vec![FailureDetail {
            kind: "qa_state_live_validation_unimplemented".to_string(),
            command: Some(COMMAND.to_string()),
            message: Some(
                "this implementation attempt only supports --input and --trace offline validation"
                    .to_string(),
            ),
            expected: Some(json!("--input or --trace offline validation")),
            actual: Some(json!("live target/method source")),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}
