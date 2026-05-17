//! OpenAPI spec inspection: parse strict OpenAPI 3.0 specs and the supported
//! OpenAPI 3.1 subset, then derive the facts pp needs to render native direct-HTTP CLI/MCP workspaces.

mod auth;
mod diagnostics;
mod json_pointer;
mod model;
mod operation;
mod schema;
pub(crate) use auth::{AuthPlan, AuthSelectionPolicy};
#[allow(unused_imports)]
pub(crate) use diagnostics::{SchemaFeature, UnsupportedSchemaDiagnostic};
pub(crate) use model::PpSpec;
pub(crate) use operation::{
    OperationRef, PpParameter, PpParameterLocation, PpParameterRef, PpRequestBodyRef,
};
pub(crate) use schema::{schema_projection, ProjectedSchema, SchemaPrimitive, SchemaShape};
pub(crate) mod preparation_rules;
pub(crate) mod references;
pub mod report;
pub mod slice;
pub(crate) mod transform;
pub(crate) mod traversal;

use anyhow::{anyhow, Context, Result};
use heck::ToKebabCase;
use regex::Regex;
use report::ReportEntry;
use serde::Serialize;
use std::path::Path;

/// Auth shape pp can template a wrapper for. Anything outside this set is
/// MVP-unsupported and surfaces as `AuthKind::Unsupported { reason }`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthKind {
    None,
    Bearer,
    HttpBasic,
    ApiKey { header_name: String },
    QueryApiKey { param_name: String },
    Unsupported { reason: String },
}

/// Everything pp extracts from a spec before rendering native templates.
#[derive(Debug, Clone, Serialize)]
pub struct SpecFacts {
    pub title: String,
    pub bin_name: String,
    pub base_url: Option<String>,
    pub base_url_is_relative: bool,
    pub operation_count: usize,
    pub auth_kind: AuthKind,
}

pub(crate) struct LoadedSpec {
    pub spec: PpSpec,
    pub facts: SpecFacts,
    pub auth_plan: AuthPlan,
    pub reports: Vec<ReportEntry>,
    pub transform_plan: transform::TransformPlan,
    pub preparation_warnings: Vec<String>,
}

pub(crate) struct LoadedOperationListingSpec {
    pub spec: PpSpec,
    pub reports: Vec<ReportEntry>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LoadOptions {
    pub slice: slice::SliceOptions,
    pub auth_policy: AuthSelectionPolicy,
}

/// Parse the spec at `path` (YAML or JSON, detected by content),
/// optionally slice it, and derive [`SpecFacts`].
pub(crate) fn load(path: &Path) -> Result<LoadedSpec> {
    load_with_options(path, &LoadOptions::default())
}

pub(crate) fn load_with_options(path: &Path, options: &LoadOptions) -> Result<LoadedSpec> {
    let prepared = prepare_openapi(path, options)?;
    let (facts, auth_plan) = inspect_spec(&prepared.spec, &options.auth_policy)?;
    let preparation_warnings = report::formatted_warnings(&prepared.reports);

    Ok(LoadedSpec {
        spec: prepared.spec,
        facts,
        auth_plan,
        reports: prepared.reports,
        transform_plan: prepared.transform_plan,
        preparation_warnings,
    })
}

pub(crate) fn load_for_operation_listing(
    path: &Path,
    options: &LoadOptions,
) -> Result<LoadedOperationListingSpec> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read spec: {}", path.display()))?;
    let mut reports = Vec::new();
    let mut spec = parse_prepared(&raw)
        .with_context(|| format!("failed to parse spec: {}", path.display()))?;

    if !options.slice.is_noop() {
        let slice_report = slice::slice_spec(&mut spec, &options.slice)?;
        reports.extend(slice_report.report_entries());
    }

    Ok(LoadedOperationListingSpec { spec, reports })
}

struct PreparedOpenApi {
    spec: PpSpec,
    reports: Vec<ReportEntry>,
    transform_plan: transform::TransformPlan,
}

fn prepare_openapi(path: &Path, options: &LoadOptions) -> Result<PreparedOpenApi> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read spec: {}", path.display()))?;
    let mut reports = Vec::new();
    let audits = Vec::new();
    let mut spec = parse_prepared(&raw)
        .with_context(|| format!("failed to parse spec: {}", path.display()))?;

    if !options.slice.is_noop() {
        let slice_report = slice::slice_spec(&mut spec, &options.slice)?;
        reports.extend(slice_report.report_entries());
    }

    let transform_plan = transform::TransformPlan::from_reports_with_audits(&reports, audits);

    Ok(PreparedOpenApi {
        spec,
        reports,
        transform_plan,
    })
}

/// Parse the spec at `path` (YAML or JSON, detected by extension and content)
/// and derive [`SpecFacts`].
#[allow(dead_code)]
pub fn inspect(path: &Path) -> Result<SpecFacts> {
    Ok(load(path)?.facts)
}

#[allow(dead_code)]
pub(crate) fn inspect_with_options(path: &Path, options: &LoadOptions) -> Result<SpecFacts> {
    Ok(load_with_options(path, options)?.facts)
}

fn inspect_spec(spec: &PpSpec, auth_policy: &AuthSelectionPolicy) -> Result<(SpecFacts, AuthPlan)> {
    let title = spec.title().to_string();
    let bin_name = bin_name_from_title(&title);

    let (base_url, base_url_is_relative) = match spec.first_server_url() {
        None => (None, false),
        Some(url) => {
            let is_relative = !(url.starts_with("http://") || url.starts_with("https://"));
            (Some(url.to_string()), is_relative)
        }
    };

    let operation_count = count_operations(spec);
    let auth_plan = auth::derive_auth_plan_with_policy(spec, auth_policy)?;
    let auth_kind = auth_plan.selected.clone();

    Ok((
        SpecFacts {
            title,
            bin_name,
            base_url,
            base_url_is_relative,
            operation_count,
            auth_kind,
        },
        auth_plan,
    ))
}

fn bin_name_from_title(title: &str) -> String {
    let openapi_noise = Regex::new(r"(?i)\bopen\s*api\s+\d+(\.\d+)?\b").expect("valid regex");
    let version_noise = Regex::new(r"(?i)\b(v\d+|v?\d+\.\d+(\.\d+)?)\b").expect("valid regex");
    let stripped = openapi_noise.replace_all(title, "");
    let stripped = version_noise.replace_all(&stripped, "");
    // Cargo crate names must be ASCII [a-zA-Z0-9_-]; transliterate / strip non-ASCII
    // so specs with Unicode titles (e.g. PokéAPI's "é") still produce valid crates.
    let ascii_only: String = stripped
        .chars()
        .map(|c| if c.is_ascii() { c } else { ' ' })
        .collect();
    ascii_only
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_kebab_case()
}

#[allow(dead_code)]
fn parse(raw: &str, _path: &Path) -> Result<(PpSpec, Vec<ReportEntry>)> {
    Ok((parse_prepared(raw)?, Vec::new()))
}

fn parse_prepared(raw: &str) -> Result<PpSpec> {
    // Try JSON first when the content looks like JSON for clearer parser errors.
    // Otherwise use YAML, which also accepts JSON-like syntax.
    let trimmed = raw.trim_start();
    let doc: serde_json::Value = if trimmed.starts_with('{') {
        serde_json::from_str(raw).map_err(|e| anyhow!("JSON parse error: {e}"))?
    } else {
        serde_yaml::from_str(raw).map_err(|e| anyhow!("YAML parse error: {e}"))?
    };
    validate_openapi_document(&doc, raw)?;
    Ok(PpSpec::new(doc))
}

fn validate_openapi_document(doc: &serde_json::Value, raw: &str) -> Result<()> {
    let version = doc
        .get("openapi")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("OpenAPI document is missing string field 'openapi'"))?;
    if version.starts_with("3.0.") {
        // Keep the existing strict 3.0 parser contract: 3.0 input must still satisfy
        // the openapiv3 crate rather than being accepted only by pp's subset reader.
        let _: openapiv3::OpenAPI = if raw.trim_start().starts_with('{') {
            serde_json::from_str(raw).map_err(|e| anyhow!("OpenAPI 3.0 parse error: {e}"))?
        } else {
            serde_yaml::from_str(raw).map_err(|e| anyhow!("OpenAPI 3.0 parse error: {e}"))?
        };
    } else if !version.starts_with("3.1.") {
        anyhow::bail!(
            "unsupported OpenAPI version '{version}'; pp supports 3.0.x and the safe 3.1.x subset"
        );
    }
    if doc
        .pointer("/info/title")
        .and_then(serde_json::Value::as_str)
        .is_none()
    {
        anyhow::bail!("OpenAPI document is missing string field info.title");
    }
    if !doc.get("paths").map(|v| v.is_object()).unwrap_or(false) {
        anyhow::bail!("OpenAPI document is missing object field 'paths'");
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn parse_spec_for_tests(raw: &str) -> Result<PpSpec> {
    parse_prepared(raw)
}

fn count_operations(spec: &PpSpec) -> usize {
    spec.operation_count()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PETSTORE_MINIMAL: &str = r#"
openapi: 3.0.0
info:
  title: Swagger Petstore
  version: "1.0.0"
servers:
  - url: https://petstore3.swagger.io/api/v3
paths:
  /pet/findByStatus:
    get:
      operationId: findPetsByStatus
      responses:
        '200':
          description: ok
"#;

    #[test]
    fn petstore_inspects_cleanly() {
        let spec = parse_spec_for_tests(PETSTORE_MINIMAL).unwrap();
        let facts = inspect_spec(&spec, &AuthSelectionPolicy::default())
            .unwrap()
            .0;
        assert_eq!(facts.bin_name, "swagger-petstore");
        assert_eq!(facts.operation_count, 1);
        assert_eq!(facts.auth_kind, AuthKind::None);
        assert_eq!(
            facts.base_url.as_deref(),
            Some("https://petstore3.swagger.io/api/v3")
        );
    }

    #[test]
    fn bin_name_strips_version_noise() {
        assert_eq!(
            bin_name_from_title("Swagger Petstore - OpenAPI 3.0"),
            "swagger-petstore"
        );
        assert_eq!(
            bin_name_from_title("GitHub v3 REST API"),
            "git-hub-rest-api"
        );
        assert_eq!(bin_name_from_title("My API v1.2.3"), "my-api");
        assert_eq!(bin_name_from_title("Cool API"), "cool-api");
        assert_eq!(bin_name_from_title("PokéAPI"), "pok-api");
        assert_eq!(bin_name_from_title("Über API"), "ber-api");
    }

    #[test]
    fn raw_parse_returns_no_reports_for_clean_specs() {
        let (_spec, reports) = parse(PETSTORE_MINIMAL, Path::new("petstore.yaml")).unwrap();
        assert!(reports.is_empty());
    }
}
