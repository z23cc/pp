use anyhow::Result;
use heck::ToKebabCase;
use openapiv3::{
    OpenAPI, Parameter, ParameterData, ParameterSchemaOrContent, ReferenceOr, RequestBody,
};
use serde::Serialize;
use serde_json::{json, Map, Value};

use super::response::reject_reserved_arg;
use super::schema::schema_json;

#[derive(Debug, Clone, Serialize)]
pub struct McpArg {
    pub json_name: String,
    pub binding: McpArgBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpArgBinding {
    CliFlag { cli_name: String },
    FlattenedBodyField,
    WholeJsonBody,
}

impl McpArg {
    pub(super) fn cli_flag(json_name: String, cli_name: String) -> Self {
        Self {
            json_name,
            binding: McpArgBinding::CliFlag { cli_name },
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

fn reject_cli_arg_collision(
    args: &[McpArg],
    cli_name: &str,
    json_name: &str,
    tool_name: &str,
    operation_id: &str,
    source: &str,
) -> Result<()> {
    if cli_name == "json-body" {
        anyhow::bail!(
            "MCP CLI argument collision for tool '{tool_name}' (operationId '{operation_id}'): argument '{json_name}' from {source} maps to reserved generated flag '--json-body'"
        );
    }
    if let Some(existing) = args.iter().find(|arg| {
        matches!(
            &arg.binding,
            McpArgBinding::CliFlag { cli_name: existing_cli_name } if existing_cli_name == cli_name
        )
    }) {
        anyhow::bail!(
            "MCP CLI argument collision for tool '{tool_name}' (operationId '{operation_id}'): argument '{json_name}' from {source} maps to '--{cli_name}', already used by argument '{}'",
            existing.json_name
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
        return Ok(());
    };
    let (data, is_path) = match parameter {
        Parameter::Query { parameter_data, .. } => (parameter_data, false),
        Parameter::Path { parameter_data, .. } => (parameter_data, true),
        _ => return Ok(()),
    };
    reject_reserved_arg(&data.name, tool_name, operation_id)?;
    reject_duplicate_arg(
        properties,
        &data.name,
        tool_name,
        operation_id,
        "OpenAPI parameter",
    )?;
    let cli_name = data.name.to_kebab_case();
    reject_cli_arg_collision(
        args,
        &cli_name,
        &data.name,
        tool_name,
        operation_id,
        "OpenAPI parameter",
    )?;
    let schema = parameter_schema(data, api);
    properties.insert(data.name.clone(), schema);
    if is_path || data.required {
        required.push(data.name.clone());
    }
    args.push(McpArg::cli_flag(data.name.clone(), cli_name));
    Ok(())
}

fn parameter_schema(data: &ParameterData, api: &OpenAPI) -> Value {
    match &data.format {
        ParameterSchemaOrContent::Schema(schema) => schema_json(schema, api),
        ParameterSchemaOrContent::Content(_) => json!({ "type": "string" }),
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
    let Some(ReferenceOr::Item(body)) = request_body else {
        return Ok(());
    };
    let Some(media_type) = body
        .content
        .get("application/json")
        .or_else(|| body.content.values().next())
    else {
        return Ok(());
    };
    let Some(schema) = media_type.schema.as_ref() else {
        return Ok(());
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
