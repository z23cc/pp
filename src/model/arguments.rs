use anyhow::{bail, Result};
use openapiv3::{
    OpenAPI, Parameter, ParameterData, ParameterSchemaOrContent, PathStyle, QueryStyle,
    ReferenceOr, RequestBody,
};
use serde::Serialize;
use serde_json::{Map, Value};

use super::response::reject_reserved_arg;
use super::schema::schema_json;

pub(super) const DIRECT_UNSUPPORTED_PREFIX: &str = "MCP direct HTTP invocation does not support";

#[derive(Debug, Clone, Serialize)]
pub struct McpArg {
    pub json_name: String,
    pub binding: McpArgBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpArgBinding {
    PathParam { wire_name: String },
    QueryParam { wire_name: String },
    FlattenedBodyField,
    WholeJsonBody,
}

impl McpArg {
    pub(super) fn path_param(json_name: String, wire_name: String) -> Self {
        Self {
            json_name,
            binding: McpArgBinding::PathParam { wire_name },
        }
    }

    pub(super) fn query_param(json_name: String, wire_name: String) -> Self {
        Self {
            json_name,
            binding: McpArgBinding::QueryParam { wire_name },
        }
    }

    pub(super) fn flattened_body_field(json_name: String) -> Self {
        Self {
            json_name,
            binding: McpArgBinding::FlattenedBodyField,
        }
    }

    pub(super) fn whole_json_body(json_name: String) -> Self {
        Self {
            json_name,
            binding: McpArgBinding::WholeJsonBody,
        }
    }
}

fn reject_duplicate_arg(
    properties: &Map<String, Value>,
    name: &str,
    tool_name: &str,
    operation_id: &str,
    source: &str,
) -> Result<()> {
    if properties.contains_key(name) {
        anyhow::bail!(
            "MCP argument collision for tool '{tool_name}' (operationId '{operation_id}'): argument '{name}' from {source} duplicates an existing MCP argument"
        );
    }
    Ok(())
}

pub(super) fn add_parameter(
    parameter: &ReferenceOr<Parameter>,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    args: &mut Vec<McpArg>,
    api: &OpenAPI,
    tool_name: &str,
    operation_id: &str,
) -> Result<()> {
    let ReferenceOr::Item(parameter) = parameter else {
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} unresolved parameter references for tool '{tool_name}' (operationId '{operation_id}')"
        );
    };
    let (data, is_path) = match parameter {
        Parameter::Query { parameter_data, .. } => (parameter_data, false),
        Parameter::Path { parameter_data, .. } => (parameter_data, true),
        Parameter::Header { parameter_data, .. } => {
            bail!(
                "{DIRECT_UNSUPPORTED_PREFIX} header parameter '{}' for tool '{tool_name}' (operationId '{operation_id}')",
                parameter_data.name
            );
        }
        Parameter::Cookie { parameter_data, .. } => {
            bail!(
                "{DIRECT_UNSUPPORTED_PREFIX} cookie parameter '{}' for tool '{tool_name}' (operationId '{operation_id}')",
                parameter_data.name
            );
        }
    };
    reject_reserved_arg(&data.name, tool_name, operation_id)?;
    reject_duplicate_arg(
        properties,
        &data.name,
        tool_name,
        operation_id,
        "OpenAPI parameter",
    )?;
    let schema = parameter_schema(data, api, tool_name, operation_id)?;
    reject_unsupported_direct_parameter_schema(
        &schema,
        &data.name,
        tool_name,
        operation_id,
        is_path,
    )?;
    reject_unsupported_direct_parameter_serialization(
        parameter,
        &schema,
        &data.name,
        tool_name,
        operation_id,
    )?;
    properties.insert(data.name.clone(), schema);
    if is_path || data.required {
        required.push(data.name.clone());
    }
    if is_path {
        args.push(McpArg::path_param(data.name.clone(), data.name.clone()));
    } else {
        args.push(McpArg::query_param(data.name.clone(), data.name.clone()));
    }
    Ok(())
}

fn parameter_schema(
    data: &ParameterData,
    api: &OpenAPI,
    tool_name: &str,
    operation_id: &str,
) -> Result<Value> {
    match &data.format {
        ParameterSchemaOrContent::Schema(schema) => Ok(schema_json(schema, api)),
        ParameterSchemaOrContent::Content(_) => bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} content-encoded parameter '{}' for tool '{tool_name}' (operationId '{operation_id}')",
            data.name
        ),
    }
}

fn reject_unsupported_direct_parameter_schema(
    schema: &Value,
    name: &str,
    tool_name: &str,
    operation_id: &str,
    is_path: bool,
) -> Result<()> {
    let Some(schema_type) = schema.get("type").and_then(Value::as_str) else {
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} parameter '{name}' without primitive schema type for tool '{tool_name}' (operationId '{operation_id}')"
        );
    };
    match schema_type {
        "string" | "integer" | "number" | "boolean" => Ok(()),
        "array" if !is_path => {
            let item_type = schema
                .get("items")
                .and_then(|items| items.get("type"))
                .and_then(Value::as_str);
            match item_type {
                Some("string" | "integer" | "number" | "boolean") => Ok(()),
                _ => bail!(
                    "{DIRECT_UNSUPPORTED_PREFIX} non-primitive array parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
                ),
            }
        }
        "array" => bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} array path parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
        ),
        _ => bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} {schema_type} parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
        ),
    }
}

fn reject_unsupported_direct_parameter_serialization(
    parameter: &Parameter,
    schema: &Value,
    name: &str,
    tool_name: &str,
    operation_id: &str,
) -> Result<()> {
    match parameter {
        Parameter::Query {
            parameter_data,
            style,
            ..
        } => {
            if *style != QueryStyle::Form {
                bail!(
                    "{DIRECT_UNSUPPORTED_PREFIX} non-form query parameter serialization for '{name}' on tool '{tool_name}' (operationId '{operation_id}')"
                );
            }
            if schema.get("type").and_then(Value::as_str) == Some("array")
                && parameter_data.explode == Some(false)
            {
                bail!(
                    "{DIRECT_UNSUPPORTED_PREFIX} non-exploded query array parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
                );
            }
            Ok(())
        }
        Parameter::Path {
            parameter_data,
            style,
        } => {
            if *style != PathStyle::Simple || parameter_data.explode == Some(true) {
                bail!(
                    "{DIRECT_UNSUPPORTED_PREFIX} non-simple path parameter serialization for '{name}' on tool '{tool_name}' (operationId '{operation_id}')"
                );
            }
            Ok(())
        }
        Parameter::Header { .. } | Parameter::Cookie { .. } => Ok(()),
    }
}

pub(super) fn add_body(
    request_body: Option<&ReferenceOr<RequestBody>>,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    args: &mut Vec<McpArg>,
    api: &OpenAPI,
    tool_name: &str,
    operation_id: &str,
) -> Result<()> {
    let Some(request_body) = request_body else {
        return Ok(());
    };
    let ReferenceOr::Item(body) = request_body else {
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} unresolved requestBody references for tool '{tool_name}' (operationId '{operation_id}')"
        );
    };
    let Some(media_type) = body.content.get("application/json") else {
        if body.content.is_empty() {
            return Ok(());
        }
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} non-JSON request bodies for tool '{tool_name}' (operationId '{operation_id}')"
        );
    };
    let Some(schema) = media_type.schema.as_ref() else {
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} schemaless JSON request body for tool '{tool_name}' (operationId '{operation_id}')"
        );
    };
    let body_schema = schema_json(schema, api);
    if let Some(object) = body_schema.as_object() {
        if object.get("type").and_then(Value::as_str) == Some("object") {
            if let Some(Value::Object(body_properties)) = object.get("properties") {
                let has_flattening_collision = body_properties
                    .keys()
                    .any(|name| properties.contains_key(name));
                if has_flattening_collision {
                    return add_synthetic_body_arg(
                        body_schema,
                        body.required,
                        properties,
                        required,
                        args,
                        tool_name,
                        operation_id,
                    );
                }

                for (name, property_schema) in body_properties {
                    reject_reserved_arg(name, tool_name, operation_id)?;
                    properties.insert(name.clone(), property_schema.clone());
                    args.push(McpArg::flattened_body_field(name.clone()));
                }
                if body.required {
                    if let Some(Value::Array(body_required)) = object.get("required") {
                        required.extend(
                            body_required
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string),
                        );
                    }
                }
                return Ok(());
            }
        }
    }
    add_synthetic_body_arg(
        body_schema,
        body.required,
        properties,
        required,
        args,
        tool_name,
        operation_id,
    )
}

fn add_synthetic_body_arg(
    body_schema: Value,
    body_required: bool,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    args: &mut Vec<McpArg>,
    tool_name: &str,
    operation_id: &str,
) -> Result<()> {
    reject_reserved_arg("body", tool_name, operation_id)?;
    reject_duplicate_arg(
        properties,
        "body",
        tool_name,
        operation_id,
        "synthetic request body argument",
    )?;
    properties.insert("body".to_string(), body_schema);
    if body_required {
        required.push("body".to_string());
    }
    args.push(McpArg::whole_json_body("body".to_string()));
    Ok(())
}
