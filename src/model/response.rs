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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpInvocationAdapterKind {
    ProgenitorCliBridge,
}

impl McpInvocationAdapterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProgenitorCliBridge => "progenitor_cli_bridge",
        }
    }

    pub fn rust_variant(self) -> &'static str {
        match self {
            Self::ProgenitorCliBridge => "InvocationAdapterKind::ProgenitorCliBridge",
        }
    }
}

impl McpInvocationAdapterContract {
    pub fn progenitor_cli_bridge() -> Self {
        Self {
            kind: McpInvocationAdapterKind::ProgenitorCliBridge,
            reason: "MCP tool calls use the Progenitor CLI bridge adapter because the current generated surface does not expose stable typed operation invocation metadata".to_string(),
        }
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
