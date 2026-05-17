//! Backend capability profile for generated native HTTP workspaces.
//!
//! The generation pipeline uses this module to describe the strict direct-HTTP
//! subset supported by generated CLI and MCP runtimes.

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

pub(crate) trait ApiBackend {
    fn capabilities(&self) -> BackendCapabilities;
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct NativeHttpBackend;

impl ApiBackend for NativeHttpBackend {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::native_http()
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
    }
}
