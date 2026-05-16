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
    SourceTransform {
        name: &'static str,
        changed: bool,
        replacement_count: usize,
    },
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
}
