use serde::Serialize;

use super::value_kind::ArgValueKind;

#[cfg(test)]
pub(super) use super::diagnostics::DIRECT_UNSUPPORTED_PREFIX;

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
