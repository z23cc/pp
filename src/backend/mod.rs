//! Backend adapters for API crate generation.
//!
//! The pipeline talks to this module instead of invoking a concrete codegen
//! driver directly. The initial backend is progenitor-only; the seam exists so
//! future generation modes stay isolated from spec inspection and rendering.

use anyhow::Result;
use openapiv3::OpenAPI;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackendCapabilities {
    pub profile: BackendCapabilityProfile,
    pub direct_invocation: DirectInvocationRequirements,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendCapabilityProfile {
    Progenitor,
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
    pub(crate) const fn progenitor() -> Self {
        Self {
            profile: BackendCapabilityProfile::Progenitor,
            direct_invocation: DirectInvocationRequirements {
                requirement_id: "mcp.direct_http.invocation",
                invocation_requirement:
                    "MCP runtime uses direct HTTP invocation from generated operation method/path metadata",
                supported_operation_requirement:
                    "MCP direct HTTP invocation currently supports primitive path/query parameters and JSON request bodies; query arrays are excluded until the backend emits buildable generated CLI code for them",
                parameters: DirectInvocationParameterRequirements {
                    supported_locations: &[
                        DirectInvocationParameterLocation::Path,
                        DirectInvocationParameterLocation::Query,
                    ],
                    primitive_schema_types: &["string", "integer", "number", "boolean"],
                    supports_query_arrays: false,
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

pub(crate) struct ApiCrateRequest<'a> {
    pub api: &'a OpenAPI,
    pub out_dir: &'a Path,
    pub crate_name: &'a str,
}

pub(crate) trait ApiBackend {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> BackendCapabilities;
    fn generate_api_crate(&self, request: ApiCrateRequest<'_>) -> Result<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ProgenitorBackend;

impl ApiBackend for ProgenitorBackend {
    fn name(&self) -> &'static str {
        "progenitor"
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::progenitor()
    }

    fn generate_api_crate(&self, request: ApiCrateRequest<'_>) -> Result<()> {
        crate::progenitor_driver::generate(request.api, request.out_dir, request.crate_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progenitor_backend_has_stable_diagnostic_name() {
        assert_eq!(ProgenitorBackend.name(), "progenitor");
    }

    #[test]
    fn progenitor_backend_advertises_current_codegen_limits() {
        let capabilities = ProgenitorBackend.capabilities();

        assert_eq!(capabilities.profile, BackendCapabilityProfile::Progenitor);
        assert_eq!(
            capabilities.direct_invocation.requirement_id,
            "mcp.direct_http.invocation"
        );
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
            !capabilities
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
    }
}
