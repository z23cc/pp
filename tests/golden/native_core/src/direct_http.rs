use crate::invoke::{ArgBinding, OperationInvocation, OperationInvocationResult};
use rmcp::ErrorData as McpError;
use serde_json::{json, Value};

pub(crate) struct RequestParts {
    pub path: String,
    pub query: Vec<(&'static str, Value)>,
    pub body: Option<Value>,
}

pub(crate) fn build_request_parts(
    invocation: &OperationInvocation,
) -> Result<RequestParts, McpError> {
    let mut path = invocation.path_template.to_string();
    let mut query = Vec::new();
    let mut flattened_body = serde_json::Map::new();
    let mut whole_body = None;

    for arg in invocation.args {
        let Some(value) = invocation.arguments.get(arg.json_name) else {
            continue;
        };
        match arg.binding {
            ArgBinding::PathParam { wire_name } => {
                if value.is_null() {
                    continue;
                }
                let encoded = encode_path_value(value)?;
                let placeholder = format!("{}{}{}", "{", wire_name, "}");
                if !path.contains(&placeholder) {
                    return Err(McpError::invalid_params(
                        format!("path parameter '{wire_name}' is not present in path template"),
                        None,
                    ));
                }
                path = path.replace(&placeholder, &encoded);
            }
            ArgBinding::QueryParam { wire_name } => {
                if value.is_null() {
                    continue;
                }
                query.push((wire_name, value.clone()))
            }
            ArgBinding::FlattenedJsonBodyField => {
                flattened_body.insert(arg.json_name.to_string(), value.clone());
            }
            ArgBinding::WholeJsonBody => {
                whole_body = Some(value.clone());
            }
        }
    }

    if path.contains('{') || path.contains('}') {
        return Err(McpError::invalid_params(
            format!("missing path parameter for template '{}'", invocation.path_template),
            None,
        ));
    }

    let body = whole_body.or_else(|| {
        if flattened_body.is_empty() {
            None
        } else {
            Some(Value::Object(flattened_body))
        }
    });

    Ok(RequestParts { path, query, body })
}

pub(crate) fn build_url(
    base_url: &str,
    path: &str,
    query: &[(&'static str, Value)],
) -> Result<reqwest::Url, String> {
    let mut base = base_url.trim_end_matches('/').to_string();
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    base.push_str(&path);
    let mut url = reqwest::Url::parse(&base)
        .map_err(|error| format!("invalid operation URL: {error}"))?;
    let mut query_pairs = Vec::new();
    for (name, value) in query {
        collect_query_pairs(&mut query_pairs, name, value);
    }
    if !query_pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(query_pairs);
    }
    Ok(url)
}

pub(crate) fn transport_error(body: String) -> OperationInvocationResult {
    OperationInvocationResult {
        value: json!({
            "error": {
                "kind": "communication_error",
                "status": null,
                "body": body,
                "headers": {},
            }
        }),
        is_error: true,
    }
}

pub(crate) fn response_body_error(
    status: reqwest::StatusCode,
    headers: Value,
    body: String,
) -> OperationInvocationResult {
    OperationInvocationResult {
        value: json!({
            "error": {
                "kind": "response_body_error",
                "status": status.as_u16(),
                "body": body,
                "headers": headers,
            }
        }),
        is_error: true,
    }
}

pub(crate) fn success_response(text: &str) -> OperationInvocationResult {
    OperationInvocationResult {
        value: parse_success_body_value(text),
        is_error: false,
    }
}

pub(crate) fn error_response(
    status: reqwest::StatusCode,
    headers: Value,
    text: &str,
) -> OperationInvocationResult {
    OperationInvocationResult {
        value: json!({
            "error": {
                "kind": "error_response",
                "status": status.as_u16(),
                "body": parse_body_value(text),
                "headers": headers,
            }
        }),
        is_error: true,
    }
}

pub(crate) fn headers_to_json(headers: &reqwest::header::HeaderMap) -> Value {
    let mut object = serde_json::Map::new();
    for (name, value) in headers {
        object.insert(
            name.as_str().to_string(),
            value.to_str().unwrap_or_default().to_string().into(),
        );
    }
    Value::Object(object)
}

fn collect_query_pairs(pairs: &mut Vec<(String, String)>, name: &str, value: &Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_query_pairs(pairs, name, value);
            }
        }
        Value::String(value) => pairs.push((name.to_string(), value.clone())),
        Value::Number(value) => pairs.push((name.to_string(), value.to_string())),
        Value::Bool(value) => pairs.push((
            name.to_string(),
            (if *value { "true" } else { "false" }).to_string(),
        )),
        other => pairs.push((name.to_string(), other.to_string())),
    }
}

fn encode_path_value(value: &Value) -> Result<String, McpError> {
    let raw = match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => {
            return Err(McpError::invalid_params(
                "path parameters must be strings, numbers, or booleans",
                None,
            ));
        }
        Value::Null => String::new(),
    };
    Ok(percent_encode_path_segment(&raw))
}

fn percent_encode_path_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*byte as char);
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

fn parse_success_body_value(text: &str) -> Value {
    if text.is_empty() {
        json!({ "ok": true })
    } else {
        parse_body_value(text)
    }
}

fn parse_body_value(text: &str) -> Value {
    if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
    }
}