use crate::backend::{
    BackendDirectTypedInvocationStatus, BackendInvocationAdapterContract,
    BackendInvocationAdapterKind,
};
use anyhow::Result;
use serde::Serialize;
use serde_json::{json, Map, Value};

const MCP_RESERVED_ARG_PREFIX: &str = "_pp_";
const MCP_FIELD_FILTER_ARG_NAME: &str = "_pp_fields";
const MCP_COMPACT_ARG_NAME: &str = "_pp_compact";

#[derive(Debug, Clone, Serialize)]
pub struct McpInvocationAdapterContract {
    pub kind: McpInvocationAdapterKind,
    pub reason: String,
    pub direct_typed_invocation: McpDirectTypedInvocationStatus,
    pub requires_generated_cli_command: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpInvocationAdapterKind {
    DirectHttp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpDirectTypedInvocationStatus {
    Supported,
}

impl McpDirectTypedInvocationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
        }
    }

    pub fn rust_variant(self) -> &'static str {
        match self {
            Self::Supported => "DirectTypedInvocationStatus::Supported",
        }
    }
}

impl McpInvocationAdapterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DirectHttp => "direct_http",
        }
    }

    pub fn rust_variant(self) -> &'static str {
        match self {
            Self::DirectHttp => "InvocationAdapterKind::DirectHttp",
        }
    }
}

impl From<BackendInvocationAdapterContract> for McpInvocationAdapterContract {
    fn from(contract: BackendInvocationAdapterContract) -> Self {
        Self {
            kind: match contract.kind {
                BackendInvocationAdapterKind::DirectHttp => McpInvocationAdapterKind::DirectHttp,
            },
            reason: contract.reason,
            direct_typed_invocation: match contract.direct_typed_invocation {
                BackendDirectTypedInvocationStatus::Supported => {
                    McpDirectTypedInvocationStatus::Supported
                }
            },
            requires_generated_cli_command: contract.requires_generated_cli_command,
        }
    }
}

impl McpInvocationAdapterContract {
    pub fn direct_http() -> Self {
        BackendInvocationAdapterContract::direct_http().into()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpResponseShaping {
    pub field_filter: McpResponseShapingArg,
    pub compact: McpResponseShapingArg,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpResponseShapingArg {
    pub json_name: String,
    pub schema: Value,
    pub invalid_type_message: String,
}

impl Default for McpResponseShaping {
    fn default() -> Self {
        mcp_response_shaping()
    }
}

pub(crate) fn mcp_response_shaping() -> McpResponseShaping {
    let field_filter_message = "_pp_fields must be an array of dot paths".to_string();
    let compact_message = "_pp_compact must be a boolean".to_string();
    McpResponseShaping {
        field_filter: McpResponseShapingArg {
            json_name: MCP_FIELD_FILTER_ARG_NAME.to_string(),
            schema: json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "MCP-only response shaping: keep only these object dot paths."
            }),
            invalid_type_message: field_filter_message,
        },
        compact: McpResponseShapingArg {
            json_name: MCP_COMPACT_ARG_NAME.to_string(),
            schema: json!({
                "type": "boolean",
                "description": "MCP-only response shaping: remove nulls and empty arrays/objects from successful structured results."
            }),
            invalid_type_message: compact_message,
        },
    }
}

pub(super) fn add_mcp_reserved_properties(properties: &mut Map<String, Value>) {
    let shaping = mcp_response_shaping();
    properties.insert(shaping.field_filter.json_name, shaping.field_filter.schema);
    properties.insert(shaping.compact.json_name, shaping.compact.schema);
}

pub(super) fn reject_reserved_arg(name: &str, tool_name: &str, operation_id: &str) -> Result<()> {
    if name.starts_with(MCP_RESERVED_ARG_PREFIX) {
        anyhow::bail!(
            "OpenAPI argument '{name}' for MCP tool '{tool_name}' (operationId '{operation_id}') uses reserved pp namespace '{MCP_RESERVED_ARG_PREFIX}'"
        );
    }
    Ok(())
}
