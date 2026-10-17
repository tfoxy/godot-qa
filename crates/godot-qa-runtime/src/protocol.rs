// Extracted from lib.rs; included by lib.rs to preserve the original crate namespace.

pub fn build_request(
    command: &str,
    params: Map<String, Value>,
    request_id: Option<&str>,
    token: Option<&str>,
) -> Value {
    let mut request = Map::from_iter([
        ("protocol_version".to_string(), json!(PROTOCOL_VERSION)),
        ("command".to_string(), json!(command)),
        ("params".to_string(), Value::Object(params)),
    ]);
    if let Some(token) = token {
        request.insert("token".to_string(), json!(token));
    }
    if let Some(request_id) = request_id {
        request.insert("id".to_string(), json!(request_id));
    }
    Value::Object(request)
}

pub fn validate_response(command: &str, payload: &Value) -> Result<(), ProtocolError> {
    let object = payload.as_object().ok_or_else(|| ProtocolError {
        failure: protocol_failure(
            command,
            "live session response must be a JSON object",
            json!("JSON object response"),
            json!(type_name(payload)),
        ),
    })?;
    if !matches!(object.get("ok"), Some(Value::Bool(_))) {
        return Err(ProtocolError {
            failure: protocol_failure(
                command,
                "live session response must include boolean ok",
                json!("boolean ok field"),
                json!(type_name(object.get("ok").unwrap_or(&Value::Null))),
            ),
        });
    }
    if object.get("protocol_version") != Some(&json!(PROTOCOL_VERSION)) {
        return Err(ProtocolError {
            failure: protocol_failure(
                command,
                "live session response protocol_version did not match",
                json!(PROTOCOL_VERSION),
                object
                    .get("protocol_version")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
        });
    }
    let ok = object.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if ok {
        if !object.get("data").unwrap_or(&json!({})).is_object() {
            return Err(ProtocolError {
                failure: protocol_failure(
                    command,
                    "live session success response data must be an object",
                    json!("JSON object data field"),
                    json!(type_name(object.get("data").unwrap_or(&Value::Null))),
                ),
            });
        }
    } else if !object.get("error").unwrap_or(&Value::Null).is_object() {
        return Err(ProtocolError {
            failure: protocol_failure(
                command,
                "live session error response must include an error object",
                json!("JSON object error field"),
                json!(type_name(object.get("error").unwrap_or(&Value::Null))),
            ),
        });
    }
    Ok(())
}


fn protocol_failure(command: &str, message: &str, expected: Value, actual: Value) -> FailureDetail {
    FailureDetail {
        kind: "protocol_error".to_string(),
        command: Some(command.to_string()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}


fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

fn io_error_name(error: &std::io::Error) -> &'static str {
    match error.kind() {
        std::io::ErrorKind::ConnectionRefused => "ConnectionRefusedError",
        std::io::ErrorKind::PermissionDenied => "PermissionDenied",
        std::io::ErrorKind::TimedOut => "TimeoutError",
        _ => "OSError",
    }
}

fn live_request_id() -> String {
    format!(
        "request-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}


