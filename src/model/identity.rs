use crate::backend::DirectInvocationRequirements;
use crate::spec::traversal;
use anyhow::Result;
use heck::ToSnakeCase;
use openapiv3::OpenAPI;
use serde_json::{json, Map};
use std::collections::BTreeMap;

use super::arguments::{add_body, add_parameter, McpArgumentContext, DIRECT_UNSUPPORTED_PREFIX};
use super::response::add_mcp_reserved_properties;
use super::{McpTool, McpUnsupportedOperation};

pub(crate) struct McpModel {
    pub tools: Vec<McpTool>,
    pub unsupported_operations: Vec<McpUnsupportedOperation>,
}

pub(crate) fn mcp_model(
    api: &OpenAPI,
    auth_env_var: Option<&str>,
    capabilities: &DirectInvocationRequirements,
) -> Result<McpModel> {
    let mut tools = Vec::new();
    let mut unsupported_operations = Vec::new();
    let mut ctx = McpBuildContext {
        auth_env_var,
        api,
        capabilities,
        seen_tool_names: BTreeMap::new(),
    };
    for operation in traversal::operations(api) {
        match build_operation(operation, &mut ctx) {
            Ok(tool) => tools.push(tool),
            Err(error) => {
                let reason = error.to_string();
                if reason.starts_with(DIRECT_UNSUPPORTED_PREFIX) {
                    unsupported_operations.push(McpUnsupportedOperation {
                        operation_id: traversal::explicit_operation_id(operation.operation)
                            .map(str::to_string),
                        method: operation.method_uppercase.to_string(),
                        path: operation.path.to_string(),
                        reason,
                    });
                } else {
                    return Err(error);
                }
            }
        }
    }
    Ok(McpModel {
        tools,
        unsupported_operations,
    })
}

struct McpBuildContext<'a> {
    auth_env_var: Option<&'a str>,
    api: &'a OpenAPI,
    capabilities: &'a DirectInvocationRequirements,
    seen_tool_names: BTreeMap<String, String>,
}

fn build_operation(
    operation_ref: traversal::OperationRef<'_>,
    ctx: &mut McpBuildContext<'_>,
) -> Result<McpTool> {
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
    reject_reserved_operation_name(&name, &raw_name)?;
    let derived_description = format!("{method} {path}");
    let mut description = operation
        .summary
        .as_deref()
        .or(operation.description.as_deref())
        .unwrap_or(&derived_description)
        .chars()
        .take(1024)
        .collect::<String>();
    if let Some(auth_env_var) = ctx.auth_env_var {
        description.push_str(&format!(" [auth: {auth_env_var} env var]"));
    }

    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut args = Vec::new();

    let arg_ctx = McpArgumentContext {
        api: ctx.api,
        capabilities: ctx.capabilities,
        tool_name: &name,
        operation_id: &raw_name,
    };
    for parameter in path_params.iter().chain(operation.parameters.iter()) {
        add_parameter(
            parameter,
            &mut properties,
            &mut required,
            &mut args,
            &arg_ctx,
        )?;
    }
    add_body(
        operation.request_body.as_ref(),
        &mut properties,
        &mut required,
        &mut args,
        &arg_ctx,
    )?;
    add_mcp_reserved_properties(&mut properties);

    let schema = json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    });

    let input_schema = serde_json::to_string(&schema).expect("schema serializes");
    if let Some(previous_operation_id) = ctx.seen_tool_names.insert(name.clone(), raw_name.clone())
    {
        anyhow::bail!(
            "MCP tool name collision: operationId '{previous_operation_id}' and operationId '{raw_name}' both produce MCP tool '{name}'"
        );
    }
    Ok(McpTool {
        name,
        description,
        input_schema,
        method: method.to_string(),
        path_template: path.to_string(),
        args,
    })
}

fn reject_reserved_operation_name(name: &str, operation_id: &str) -> Result<()> {
    if matches!(name, "mcp" | "help") {
        anyhow::bail!(
            "operationId '{operation_id}' produces reserved generated CLI command '{name}'"
        );
    }
    Ok(())
}

fn operation_name(operation_id: &str) -> String {
    operation_id.to_snake_case()
}
