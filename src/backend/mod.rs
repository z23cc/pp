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
    pub precondition: &'static str,
    pub upstream_assumption: &'static str,
    pub upstream_version: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceTransformPurpose {
    ClapParserCompatibility,
    ErrorDiagnostics,
}

pub(crate) struct ApiCrateRequest<'a> {
    pub api: &'a OpenAPI,
    pub out_dir: &'a Path,
    pub crate_name: &'a str,
}

pub(crate) trait ApiBackend {
    fn name(&self) -> &'static str;
    fn generate_api_crate(&self, request: ApiCrateRequest<'_>) -> Result<ApiCrateOutput>;
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ProgenitorBackend;

impl ApiBackend for ProgenitorBackend {
    fn name(&self) -> &'static str {
        "progenitor"
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
    fn source_transform_diagnostic_carries_semantic_metadata() {
        let diagnostic = BackendDiagnostic::SourceTransform(SourceTransformDiagnostic {
            name: "example",
            changed: true,
            replacement_count: 1,
            purpose: SourceTransformPurpose::ErrorDiagnostics,
            precondition: "generated source contains fallback arm",
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
            source_transform.precondition,
            "generated source contains fallback arm"
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
