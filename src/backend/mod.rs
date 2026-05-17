//! Backend adapters for API crate generation.
//!
//! The pipeline talks to this module instead of invoking a concrete codegen
//! driver directly. The initial backend is progenitor-only; the seam exists so
//! backend diagnostics and future generation modes stay isolated from spec
//! normalization and rendering.

use anyhow::Result;
use openapiv3::OpenAPI;
use std::path::Path;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ApiCrateOutput {
    pub diagnostics: Vec<BackendDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BackendDiagnostic {
    SourceTransform(SourceTransformDiagnostic),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceTransformDiagnostic {
    pub name: &'static str,
    pub changed: bool,
    pub replacement_count: usize,
    pub purpose: SourceTransformPurpose,
    pub required: SourceTransformRequiredness,
    pub status: SourceTransformStatus,
    pub precondition: &'static str,
    pub postcondition: &'static str,
    pub upstream_assumption: &'static str,
    pub upstream_version: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceTransformPurpose {
    ClapParserCompatibility,
    ErrorDiagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceTransformRequiredness {
    Required,
    Conditional,
}

impl SourceTransformRequiredness {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Conditional => "conditional",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceTransformStatus {
    Applied,
    VerifiedNotNeeded,
    NotApplicable,
}

impl SourceTransformStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::VerifiedNotNeeded => "verified_not_needed",
            Self::NotApplicable => "not_applicable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackendCapabilities {
    pub profile: BackendCapabilityProfile,
    pub request_bodies: RequestBodyRequirements,
    pub responses: ResponseRequirements,
    pub message_content: MessageContentRequirements,
    pub parameters: ParameterRequirements,
    pub schemas: SchemaRequirements,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendCapabilityProfile {
    Progenitor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequestBodyRequirements {
    pub supported_content_types: &'static [&'static str],
    pub accepts_schemaless: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResponseRequirements {
    pub requires_single_variant_per_operation: bool,
    pub requires_relaxed_schemas: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MessageContentRequirements {
    pub requires_single_content_type_per_message: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParameterRequirements {
    pub supports_deep_object_query: bool,
    pub supports_optional_object_query: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchemaRequirements {
    pub supported_types: &'static [&'static str],
    pub supports_defaults: bool,
    pub requires_unique_sanitized_enum_variants: bool,
    pub requires_unique_sanitized_object_properties: bool,
}

impl BackendCapabilities {
    pub(crate) const fn progenitor() -> Self {
        Self {
            profile: BackendCapabilityProfile::Progenitor,
            request_bodies: RequestBodyRequirements {
                supported_content_types: &[
                    "application/json",
                    "application/x-www-form-urlencoded",
                    "application/octet-stream",
                ],
                accepts_schemaless: false,
            },
            responses: ResponseRequirements {
                requires_single_variant_per_operation: true,
                requires_relaxed_schemas: true,
            },
            message_content: MessageContentRequirements {
                requires_single_content_type_per_message: true,
            },
            parameters: ParameterRequirements {
                supports_deep_object_query: false,
                supports_optional_object_query: false,
            },
            schemas: SchemaRequirements {
                supported_types: &["string", "number", "integer", "boolean", "array", "object"],
                supports_defaults: false,
                requires_unique_sanitized_enum_variants: true,
                requires_unique_sanitized_object_properties: true,
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
    fn generate_api_crate(&self, request: ApiCrateRequest<'_>) -> Result<ApiCrateOutput>;
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

    fn generate_api_crate(&self, request: ApiCrateRequest<'_>) -> Result<ApiCrateOutput> {
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
            capabilities.request_bodies.supported_content_types,
            &[
                "application/json",
                "application/x-www-form-urlencoded",
                "application/octet-stream"
            ]
        );
        assert_eq!(
            capabilities.schemas.supported_types,
            &["string", "number", "integer", "boolean", "array", "object"]
        );
        assert!(capabilities.responses.requires_single_variant_per_operation);
        assert!(
            capabilities
                .message_content
                .requires_single_content_type_per_message
        );
        assert!(!capabilities.parameters.supports_deep_object_query);
        assert!(!capabilities.parameters.supports_optional_object_query);
        assert!(!capabilities.schemas.supports_defaults);
        assert!(!capabilities.request_bodies.accepts_schemaless);
        assert!(capabilities.schemas.requires_unique_sanitized_enum_variants);
        assert!(
            capabilities
                .schemas
                .requires_unique_sanitized_object_properties
        );
        assert!(capabilities.responses.requires_relaxed_schemas);
    }

    #[test]
    fn source_transform_diagnostic_carries_semantic_metadata() {
        let diagnostic = BackendDiagnostic::SourceTransform(SourceTransformDiagnostic {
            name: "example",
            changed: true,
            replacement_count: 1,
            purpose: SourceTransformPurpose::ErrorDiagnostics,
            required: SourceTransformRequiredness::Required,
            status: SourceTransformStatus::Applied,
            precondition: "generated source contains fallback arm",
            postcondition: "fallback arm includes response body text",
            upstream_assumption: "upstream error hides body details",
            upstream_version: "example upstream version",
        });

        let BackendDiagnostic::SourceTransform(source_transform) = diagnostic;
        assert_eq!(source_transform.name, "example");
        assert_eq!(
            source_transform.purpose,
            SourceTransformPurpose::ErrorDiagnostics
        );
        assert_eq!(
            source_transform.required,
            SourceTransformRequiredness::Required
        );
        assert_eq!(source_transform.status, SourceTransformStatus::Applied);
        assert_eq!(
            source_transform.precondition,
            "generated source contains fallback arm"
        );
        assert_eq!(
            source_transform.postcondition,
            "fallback arm includes response body text"
        );
        assert_eq!(
            source_transform.upstream_assumption,
            "upstream error hides body details"
        );
        assert_eq!(
            source_transform.upstream_version,
            "example upstream version"
        );
    }
}
