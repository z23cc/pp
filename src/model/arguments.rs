use crate::backend::DirectInvocationRequirements;
use crate::spec::{PpParameterRef, PpRequestBodyRef, PpSpec};
use crate::support::diagnostics::direct_http as direct_codes;
use anyhow::Result;
use serde::Serialize;
use serde_json::{Map, Value};

use super::diagnostics::DirectInvocationUnsupported;
use super::direct_http_plan::{
    plan_parameter, plan_request_body, DirectHttpPlanContext, PlannedParameterBinding,
    PlannedRequestBody,
};
use super::response::reject_reserved_arg;
use super::value_kind::ArgValueKind;

#[cfg(test)]
pub(super) use super::diagnostics::DIRECT_UNSUPPORTED_PREFIX;

fn unsupported_error(code: &'static str, detail: impl Into<String>) -> anyhow::Error {
    DirectInvocationUnsupported::new(code, detail).into()
}

pub(super) struct McpArgumentContext<'a> {
    pub spec: &'a PpSpec,
    pub capabilities: &'a DirectInvocationRequirements,
    pub tool_name: &'a str,
    pub operation_id: &'a str,
}

impl<'a> McpArgumentContext<'a> {
    fn direct_http_plan_context(&self) -> DirectHttpPlanContext<'a> {
        DirectHttpPlanContext {
            spec: self.spec,
            capabilities: self.capabilities,
            tool_name: self.tool_name,
            operation_id: self.operation_id,
        }
    }
}

pub type McpArg = GeneratedArg;
pub type McpArgBinding = GeneratedArgBinding;

#[derive(Debug, Clone, Serialize)]
pub struct GeneratedArg {
    pub json_name: String,
    pub binding: GeneratedArgBinding,
    pub required: bool,
    pub value_kind: ArgValueKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GeneratedArgBinding {
    PathParam { wire_name: String },
    QueryParam { wire_name: String },
    FlattenedBodyField,
    WholeJsonBody,
}

impl GeneratedArg {
    pub(super) fn path_param(
        json_name: String,
        wire_name: String,
        required: bool,
        value_kind: ArgValueKind,
    ) -> Self {
        Self {
            json_name,
            binding: GeneratedArgBinding::PathParam { wire_name },
            required,
            value_kind,
        }
    }

    pub(super) fn query_param(
        json_name: String,
        wire_name: String,
        required: bool,
        value_kind: ArgValueKind,
    ) -> Self {
        Self {
            json_name,
            binding: GeneratedArgBinding::QueryParam { wire_name },
            required,
            value_kind,
        }
    }

    pub(super) fn flattened_body_field(
        json_name: String,
        required: bool,
        value_kind: ArgValueKind,
    ) -> Self {
        Self {
            json_name,
            binding: GeneratedArgBinding::FlattenedBodyField,
            required,
            value_kind,
        }
    }

    pub(super) fn whole_json_body(json_name: String, required: bool) -> Self {
        Self {
            json_name,
            binding: GeneratedArgBinding::WholeJsonBody,
            required,
            value_kind: ArgValueKind::Json,
        }
    }
}

fn reject_reserved_cli_arg(name: &str, tool_name: &str, operation_id: &str) -> Result<()> {
    if matches!(name, "json" | "help") {
        anyhow::bail!(
            "MCP/CLI argument collision for tool '{tool_name}' (operationId '{operation_id}'): argument '{name}' is reserved by the generated CLI"
        );
    }
    Ok(())
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
    parameter: PpParameterRef<'_>,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    args: &mut Vec<McpArg>,
    ctx: &McpArgumentContext<'_>,
) -> Result<()> {
    let planned = plan_parameter(parameter, &ctx.direct_http_plan_context(), |name| {
        reject_reserved_arg(name, ctx.tool_name, ctx.operation_id)?;
        reject_reserved_cli_arg(name, ctx.tool_name, ctx.operation_id)?;
        reject_duplicate_arg(
            properties,
            name,
            ctx.tool_name,
            ctx.operation_id,
            "OpenAPI parameter",
        )
    })?;

    properties.insert(planned.json_name.clone(), planned.schema_json);
    if planned.required {
        required.push(planned.json_name.clone());
    }
    match planned.binding {
        PlannedParameterBinding::Path => args.push(McpArg::path_param(
            planned.json_name,
            planned.wire_name,
            planned.required,
            planned.value_kind,
        )),
        PlannedParameterBinding::Query => args.push(McpArg::query_param(
            planned.json_name,
            planned.wire_name,
            planned.required,
            planned.value_kind,
        )),
    }
    Ok(())
}

pub(super) fn add_body(
    request_body: Option<PpRequestBodyRef<'_>>,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    args: &mut Vec<McpArg>,
    ctx: &McpArgumentContext<'_>,
) -> Result<()> {
    match plan_request_body(
        request_body,
        &ctx.direct_http_plan_context(),
        |field_names| {
            if field_names.iter().any(|name| properties.contains_key(name)) {
                return Err(unsupported_error(
                    direct_codes::REQUEST_BODY_FIELD_COLLISION,
                    format!(
                        "flattened JSON request body field collision for tool '{}' (operationId '{}')",
                        ctx.tool_name, ctx.operation_id
                    ),
                ));
            }
            Ok(())
        },
        |field_name| {
            reject_reserved_arg(field_name, ctx.tool_name, ctx.operation_id)?;
            reject_reserved_cli_arg(field_name, ctx.tool_name, ctx.operation_id)
        },
    )? {
        PlannedRequestBody::None => Ok(()),
        PlannedRequestBody::Flattened {
            fields,
            required: body_required,
        } => {
            for field in fields {
                properties.insert(field.json_name.clone(), field.schema_json);
                args.push(McpArg::flattened_body_field(
                    field.json_name,
                    field.required,
                    field.value_kind,
                ));
            }
            required.extend(body_required);
            Ok(())
        }
        PlannedRequestBody::WholeJson {
            schema_json,
            required: body_required,
        } => add_synthetic_body_arg(
            schema_json,
            body_required,
            properties,
            required,
            args,
            ctx.tool_name,
            ctx.operation_id,
        ),
    }
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
    reject_reserved_cli_arg("body", tool_name, operation_id)?;
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
    args.push(McpArg::whole_json_body("body".to_string(), body_required));
    Ok(())
}
