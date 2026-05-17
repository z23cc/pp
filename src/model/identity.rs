use crate::backend::ApiBackend;
use crate::spec::{traversal, OperationRef, PpSpec};
use anyhow::Result;
use heck::ToSnakeCase;
use serde_json::json;
use std::collections::BTreeMap;

use super::diagnostics::DirectInvocationUnsupported;
use super::response::{add_mcp_reserved_properties, mcp_response_shaping};
use super::{
    GeneratedOperation, McpInvocationAdapterContract, McpSurfaceModel, McpTool,
    OperationInvocationPlanRequest, UnsupportedOperation,
};

pub(crate) struct CanonicalApiModel {
    pub operations: Vec<GeneratedOperation>,
    pub unsupported_operations: Vec<UnsupportedOperation>,
}

pub(crate) fn canonical_model<B: ApiBackend>(
    spec: &PpSpec,
    auth_env_var: Option<&str>,
    backend: &B,
) -> Result<CanonicalApiModel> {
    let mut operations = Vec::new();
    let mut unsupported_operations = Vec::new();
    let mut ctx = CanonicalBuildContext {
        auth_env_var,
        spec,
        backend,
        seen_operation_names: BTreeMap::new(),
    };
    for operation in traversal::operations(spec) {
        match build_operation(operation.clone(), &mut ctx) {
            Ok(operation) => operations.push(operation),
            Err(error) => match error.downcast::<DirectInvocationUnsupported>() {
                Ok(unsupported) => unsupported_operations.push(UnsupportedOperation {
                    operation_id: operation.explicit_operation_id().map(str::to_string),
                    method: operation.method_uppercase.to_string(),
                    path: operation.path.to_string(),
                    reason: unsupported.to_string(),
                    diagnostic_code: unsupported.code().to_string(),
                }),
                Err(error) => return Err(error),
            },
        }
    }
    Ok(CanonicalApiModel {
        operations,
        unsupported_operations,
    })
}

#[cfg(test)]
pub(crate) fn mcp_model<B: ApiBackend>(
    spec: &PpSpec,
    auth_env_var: Option<&str>,
    backend: &B,
) -> Result<McpSurfaceModel> {
    let canonical = canonical_model(spec, auth_env_var, backend)?;
    mcp_surface_model(
        canonical.operations,
        canonical.unsupported_operations,
        backend.invocation_adapter_contract().into(),
    )
}

pub(crate) fn mcp_surface_model(
    operations: Vec<GeneratedOperation>,
    unsupported_operations: Vec<UnsupportedOperation>,
    invocation_adapter: McpInvocationAdapterContract,
) -> Result<McpSurfaceModel> {
    let tools = operations
        .into_iter()
        .map(mcp_tool_from_operation)
        .collect::<Result<Vec<_>>>()?;
    Ok(McpSurfaceModel {
        tools,
        unsupported_operations,
        response_shaping: mcp_response_shaping(),
        invocation_adapter,
    })
}

struct CanonicalBuildContext<'a, B: ApiBackend> {
    auth_env_var: Option<&'a str>,
    spec: &'a PpSpec,
    backend: &'a B,
    seen_operation_names: BTreeMap<String, String>,
}

fn build_operation<B: ApiBackend>(
    operation_ref: OperationRef<'_>,
    ctx: &mut CanonicalBuildContext<'_, B>,
) -> Result<GeneratedOperation> {
    let method = operation_ref.method_uppercase;
    let path = operation_ref.path;
    let Some(raw_name) = operation_ref.explicit_operation_id().map(str::to_string) else {
        let derived_id = traversal::derived_operation_identifier(operation_ref.method, path);
        anyhow::bail!(
            "operation {method} {path} is missing operationId; explicit operationId is required for codegen/MCP identity. Add a stable operationId to this selected operation or exclude it from generation with `--exclude-operation \"{derived_id}\"`."
        );
    };
    let name = operation_name(&raw_name);
    reject_reserved_operation_name(&name, &raw_name)?;
    let derived_description = format!("{method} {path}");
    let mut description = operation_ref
        .summary_or_description()
        .unwrap_or(&derived_description)
        .chars()
        .take(1024)
        .collect::<String>();
    if let Some(auth_env_var) = ctx.auth_env_var {
        description.push_str(&format!(" [auth: {auth_env_var} env var]"));
    }

    let invocation = ctx
        .backend
        .plan_operation_invocation(OperationInvocationPlanRequest {
            spec: ctx.spec,
            operation: operation_ref,
            tool_name: &name,
            operation_id: &raw_name,
        })?;

    if let Some(previous_operation_id) = ctx
        .seen_operation_names
        .insert(name.clone(), raw_name.clone())
    {
        anyhow::bail!(
            "MCP tool name collision: operationId '{previous_operation_id}' and operationId '{raw_name}' both produce MCP tool '{name}'"
        );
    }
    Ok(GeneratedOperation {
        name,
        operation_id: raw_name,
        description,
        method: method.to_string(),
        path_template: path.to_string(),
        invocation,
    })
}

fn mcp_tool_from_operation(operation: GeneratedOperation) -> Result<McpTool> {
    let mut properties = operation.invocation.properties;
    add_mcp_reserved_properties(&mut properties);
    let schema = json!({
        "type": "object",
        "properties": properties,
        "required": operation.invocation.required,
        "additionalProperties": false,
    });
    let input_schema = serde_json::to_string(&schema).expect("schema serializes");
    Ok(McpTool {
        name: operation.name,
        description: operation.description,
        input_schema,
        method: operation.method,
        path_template: operation.path_template,
        args: operation.invocation.args,
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
