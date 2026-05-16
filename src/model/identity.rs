use crate::spec::traversal;
use anyhow::Result;
use heck::ToSnakeCase;
use openapiv3::OpenAPI;
use serde_json::{json, Map};
use std::collections::BTreeMap;

use super::arguments::{add_body, add_parameter};
use super::response::add_mcp_reserved_properties;
use super::McpTool;

pub(crate) fn mcp_tools(api: &OpenAPI, auth_env_var: Option<&str>) -> Result<Vec<McpTool>> {
    let mut tools = Vec::new();
    let mut ctx = McpBuildContext {
        auth_env_var,
        api,
        seen_tool_names: BTreeMap::new(),
    };
    for operation in traversal::operations(api) {
        push_operation(&mut tools, operation, &mut ctx)?;
    }
    Ok(tools)
}

struct McpBuildContext<'a> {
    auth_env_var: Option<&'a str>,
    api: &'a OpenAPI,
    seen_tool_names: BTreeMap<String, String>,
}

fn push_operation(
    tools: &mut Vec<McpTool>,
    operation_ref: traversal::OperationRef<'_>,
    ctx: &mut McpBuildContext<'_>,
) -> Result<()> {
    let method = operation_ref.method_uppercase;
    let path = operation_ref.path;
    let path_params = operation_ref.path_parameters;
    let operation = operation_ref.operation;
    let Some(raw_name) = traversal::explicit_operation_id(operation).map(str::to_string) else {
        let derived_id = traversal::derived_operation_identifier(operation_ref.method, path);
        anyhow::bail!(
            "operation {method} {path} is missing operationId; explicit operationId is required for codegen/MCP identity. Add a stable operationId to this selected operation or exclude it from generation with `--exclude-operation \"{derived_id}\"`."
        );
    };
    let name = operation_name(&raw_name);
    if let Some(previous_operation_id) = ctx.seen_tool_names.insert(name.clone(), raw_name.clone())
    {
        anyhow::bail!(
            "MCP tool name collision: operationId '{previous_operation_id}' and operationId '{raw_name}' both produce MCP tool '{name}'"
        );
    }
    let fallback_description = format!("{method} {path}");
    let mut description = operation
        .summary
        .as_deref()
        .or(operation.description.as_deref())
        .unwrap_or(&fallback_description)
        .chars()
        .take(1024)
        .collect::<String>();
    if let Some(auth_env_var) = ctx.auth_env_var {
        description.push_str(&format!(" [auth: {auth_env_var} env var]"));
    }

    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut args = Vec::new();

    for parameter in path_params.iter().chain(operation.parameters.iter()) {
        add_parameter(
            parameter,
            &mut properties,
            &mut required,
            &mut args,
            ctx.api,
            &name,
            &raw_name,
        )?;
    }
    add_body(
        operation.request_body.as_ref(),
        &mut properties,
        &mut required,
        &mut args,
        ctx.api,
        &name,
        &raw_name,
    )?;
    add_mcp_reserved_properties(&mut properties);

    let schema = json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    });

    let input_schema = serde_json::to_string(&schema).expect("schema serializes");
    tools.push(McpTool {
        name,
        description,
        input_schema,
        args,
    });
    Ok(())
}

fn operation_name(operation_id: &str) -> String {
    operation_id.to_snake_case()
}
