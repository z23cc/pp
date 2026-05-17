use crate::context::Context;
use rmcp::ErrorData as McpError;
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationAdapterKind {
    DirectHttp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectTypedInvocationStatus {
    Supported,
}

impl DirectTypedInvocationStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
        }
    }
}

impl InvocationAdapterKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DirectHttp => "direct_http",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvocationAdapterContract {
    pub kind: InvocationAdapterKind,
    pub reason: &'static str,
    pub direct_typed_invocation: DirectTypedInvocationStatus,
    pub preserves_cli_dispatch: bool,
    pub uses_temp_json_body_files: bool,
    pub requires_generated_cli_command: bool,
}

pub const INVOCATION_ADAPTER_CONTRACT: InvocationAdapterContract = InvocationAdapterContract {
    kind: InvocationAdapterKind::DirectHttp,
    reason: "MCP tool calls use direct HTTP operation invocation from generated operation metadata",
    direct_typed_invocation: DirectTypedInvocationStatus::Supported,
    preserves_cli_dispatch: false,
    uses_temp_json_body_files: false,
    requires_generated_cli_command: false,
};
pub const INVOCATION_ADAPTER_KIND: InvocationAdapterKind = INVOCATION_ADAPTER_CONTRACT.kind;
pub const INVOCATION_ADAPTER_KIND_NAME: &str = INVOCATION_ADAPTER_KIND.as_str();
pub const INVOCATION_ADAPTER_REASON: &str = INVOCATION_ADAPTER_CONTRACT.reason;
pub const DIRECT_TYPED_INVOCATION_STATUS: DirectTypedInvocationStatus = INVOCATION_ADAPTER_CONTRACT.direct_typed_invocation;
pub const DIRECT_TYPED_INVOCATION_STATUS_NAME: &str = DIRECT_TYPED_INVOCATION_STATUS.as_str();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgBinding {
    PathParam { wire_name: &'static str },
    QueryParam { wire_name: &'static str },
    FlattenedJsonBodyField,
    WholeJsonBody,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArgDef {
    pub json_name: &'static str,
    pub binding: ArgBinding,
}

pub struct OperationInvocation {
    pub name: &'static str,
    pub method: &'static str,
    pub path_template: &'static str,
    pub args: &'static [ArgDef],
    pub arguments: rmcp::model::JsonObject,
}

pub struct OperationInvocationResult {
    pub value: Value,
    pub is_error: bool,
}

/// Invoke one generated operation from the MCP runtime with direct HTTP.
pub async fn invoke_operation(
    context: Context,
    invocation: OperationInvocation,
) -> Result<OperationInvocationResult, McpError> {
    validate_invocation_adapter_contract_for_tool(invocation.name, invocation.args)?;
    DirectHttpInvoker::invoke(context, invocation).await
}

pub fn validate_invocation_adapter_contract_for_tool(
    _tool_name: &str,
    _args: &[ArgDef],
) -> Result<(), McpError> {
    let contract = INVOCATION_ADAPTER_CONTRACT;
    if contract.kind != InvocationAdapterKind::DirectHttp {
        return Err(McpError::internal_error(
            format!("unsupported MCP invocation adapter: {}", contract.kind.as_str()),
            None,
        ));
    }
    if contract.direct_typed_invocation != DirectTypedInvocationStatus::Supported {
        return Err(McpError::internal_error(
            format!(
                "direct HTTP MCP invocation is not supported by this runtime contract: {}",
                contract.direct_typed_invocation.as_str()
            ),
            None,
        ));
    }
    Ok(())
}

struct DirectHttpInvoker;

impl DirectHttpInvoker {
    async fn invoke(
        context: Context,
        invocation: OperationInvocation,
    ) -> Result<OperationInvocationResult, McpError> {
        let parts = crate::direct_http::build_request_parts(&invocation)?;
        let url = match crate::direct_http::build_url(&context.base_url, &parts.path, &parts.query) {
            Ok(url) => url,
            Err(error) => return Ok(crate::direct_http::transport_error(error)),
        };
        let method = invocation
            .method
            .parse::<reqwest::Method>()
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        let debug_request = context.debug_enabled.then(|| {
            (
                Instant::now(),
                method.as_str().to_string(),
                invocation.path_template.to_string(),
            )
        });
        let mut request = context.client.request(method.clone(), url.clone());
        if let Some(body) = parts.body {
            request = request.json(&body);
        }

        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                if let Some((started_at, method, target)) = debug_request.as_ref() {
                    log_debug_request(method, target, None, started_at.elapsed());
                }
                return Ok(crate::direct_http::transport_error(error.to_string()));
            }
        };

        let status = response.status();
        if let Some((started_at, method, target)) = debug_request.as_ref() {
            log_debug_request(method, target, Some(status), started_at.elapsed());
        }
        let headers = crate::direct_http::headers_to_json(response.headers());
        let text = match response.text().await {
            Ok(text) => text,
            Err(error) => {
                return Ok(crate::direct_http::response_body_error(
                    status,
                    headers,
                    error.to_string(),
                ));
            }
        };
        if status.is_success() {
            Ok(crate::direct_http::success_response(&text))
        } else {
            Ok(crate::direct_http::error_response(status, headers, &text))
        }
    }
}

fn log_debug_request(
    method: &str,
    target: &str,
    status: Option<reqwest::StatusCode>,
    elapsed: Duration,
) {
    match status {
        Some(status) => eprintln!(
            "[pp debug] {method} {target} -> {} in {}ms",
            status.as_u16(),
            elapsed.as_millis()
        ),
        None => eprintln!(
            "[pp debug] {method} {target} -> transport_error in {}ms",
            elapsed.as_millis()
        ),
    }
}
