//! Backend interface and capability profile for generated native HTTP workspaces.
//!
//! The generation pipeline asks a backend adapter to plan operation invocation
//! instead of threading concrete direct-HTTP planning details through the model.

use crate::model::{OperationInvocationPlan, OperationInvocationPlanRequest};
use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackendCapabilities {
    pub profile: BackendCapabilityProfile,
    pub direct_invocation: DirectInvocationRequirements,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendCapabilityProfile {
    NativeHttp,
}

impl BackendCapabilityProfile {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::NativeHttp => "native_http",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirectInvocationRequirements {
    pub requirement_id: &'static str,
    pub invocation_requirement: &'static str,
    pub supported_operation_requirement: &'static str,
    pub parameters: DirectInvocationParameterRequirements,
    pub request_bodies: DirectInvocationRequestBodyRequirements,
    pub auth: DirectInvocationAuthRequirements,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirectInvocationParameterRequirements {
    pub supported_locations: &'static [DirectInvocationParameterLocation],
    pub primitive_schema_types: &'static [&'static str],
    pub supports_query_arrays: bool,
    pub supports_non_exploded_query_arrays: bool,
    pub requires_form_query_style: bool,
    pub requires_simple_path_style: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectInvocationParameterLocation {
    Path,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirectInvocationRequestBodyRequirements {
    pub json_content_type: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirectInvocationAuthRequirements {
    pub uses_runtime_auth_context: bool,
}

impl BackendCapabilities {
    pub(crate) const fn native_http() -> Self {
        Self {
            profile: BackendCapabilityProfile::NativeHttp,
            direct_invocation: DirectInvocationRequirements {
                requirement_id: "native_http.direct_http.invocation",
                invocation_requirement:
                    "native HTTP runtime uses generated operation method/path metadata for direct HTTP invocation",
                supported_operation_requirement:
                    "native HTTP direct invocation supports primitive path/query parameters, exploded primitive query arrays, and JSON request bodies",
                parameters: DirectInvocationParameterRequirements {
                    supported_locations: &[
                        DirectInvocationParameterLocation::Path,
                        DirectInvocationParameterLocation::Query,
                    ],
                    primitive_schema_types: &["string", "integer", "number", "boolean"],
                    supports_query_arrays: true,
                    supports_non_exploded_query_arrays: false,
                    requires_form_query_style: true,
                    requires_simple_path_style: true,
                },
                request_bodies: DirectInvocationRequestBodyRequirements {
                    json_content_type: "application/json",
                },
                auth: DirectInvocationAuthRequirements {
                    uses_runtime_auth_context: true,
                },
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackendInvocationAdapterContract {
    pub kind: BackendInvocationAdapterKind,
    pub reason: String,
    pub direct_typed_invocation: BackendDirectTypedInvocationStatus,
    pub requires_generated_cli_command: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendInvocationAdapterKind {
    DirectHttp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendDirectTypedInvocationStatus {
    Supported,
}

impl BackendInvocationAdapterContract {
    pub(crate) fn direct_http() -> Self {
        Self {
            kind: BackendInvocationAdapterKind::DirectHttp,
            reason: "MCP tool calls use direct HTTP operation invocation from generated operation metadata".to_string(),
            direct_typed_invocation: BackendDirectTypedInvocationStatus::Supported,
            requires_generated_cli_command: false,
        }
    }
}

pub(crate) trait ApiBackend {
    fn capabilities(&self) -> BackendCapabilities;

    fn invocation_adapter_contract(&self) -> BackendInvocationAdapterContract;

    fn plan_operation_invocation(
        &self,
        request: OperationInvocationPlanRequest<'_>,
    ) -> Result<OperationInvocationPlan>;
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct NativeHttpBackend;

impl ApiBackend for NativeHttpBackend {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::native_http()
    }

    fn invocation_adapter_contract(&self) -> BackendInvocationAdapterContract {
        BackendInvocationAdapterContract::direct_http()
    }

    fn plan_operation_invocation(
        &self,
        request: OperationInvocationPlanRequest<'_>,
    ) -> Result<OperationInvocationPlan> {
        let capabilities = self.capabilities();
        crate::model::invocation_plan::plan_native_http_operation_invocation(
            request,
            &capabilities.direct_invocation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_http_backend_advertises_direct_http_subset() {
        let capabilities = NativeHttpBackend.capabilities();

        assert_eq!(capabilities.profile, BackendCapabilityProfile::NativeHttp);
        assert_eq!(capabilities.profile.as_str(), "native_http");
        assert_eq!(
            capabilities.direct_invocation.requirement_id,
            "native_http.direct_http.invocation"
        );
        assert!(capabilities
            .direct_invocation
            .invocation_requirement
            .contains("native HTTP runtime"));
        assert_eq!(
            capabilities
                .direct_invocation
                .parameters
                .supported_locations,
            &[
                DirectInvocationParameterLocation::Path,
                DirectInvocationParameterLocation::Query,
            ]
        );
        assert_eq!(
            capabilities
                .direct_invocation
                .parameters
                .primitive_schema_types,
            &["string", "integer", "number", "boolean"]
        );
        assert!(
            capabilities
                .direct_invocation
                .parameters
                .supports_query_arrays
        );
        assert!(
            !capabilities
                .direct_invocation
                .parameters
                .supports_non_exploded_query_arrays
        );
        assert_eq!(
            capabilities
                .direct_invocation
                .request_bodies
                .json_content_type,
            "application/json"
        );
        assert!(
            capabilities
                .direct_invocation
                .auth
                .uses_runtime_auth_context
        );

        let adapter = NativeHttpBackend.invocation_adapter_contract();
        assert_eq!(adapter.kind, BackendInvocationAdapterKind::DirectHttp);
        assert_eq!(
            adapter.direct_typed_invocation,
            BackendDirectTypedInvocationStatus::Supported
        );
        assert!(!adapter.requires_generated_cli_command);
        assert!(adapter.reason.contains("direct HTTP operation invocation"));
    }
}
