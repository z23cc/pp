//! OpenAPI spec inspection: parse a strict OpenAPI 3.0 spec and derive the
//! facts pp needs to render native direct-HTTP CLI/MCP workspaces.

mod auth;
pub(crate) use auth::{AuthPlan, AuthSelectionPolicy};
pub(crate) mod preparation_rules;
pub(crate) mod references;
pub mod report;
pub mod slice;
pub(crate) mod transform;
pub(crate) mod traversal;

use anyhow::{anyhow, Context, Result};
use heck::ToKebabCase;
use openapiv3::OpenAPI;
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
    pub api: OpenAPI,
    pub facts: SpecFacts,
    pub auth_plan: AuthPlan,
    pub reports: Vec<ReportEntry>,
    pub transform_plan: transform::TransformPlan,
    pub preparation_warnings: Vec<String>,
}

pub(crate) struct LoadedOperationListingSpec {
    pub api: OpenAPI,
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
    let (facts, auth_plan) = inspect_openapi(&prepared.api, &options.auth_policy)?;
    let preparation_warnings = report::formatted_warnings(&prepared.reports);

    Ok(LoadedSpec {
        api: prepared.api,
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
    let mut api = parse_prepared(&raw)
        .with_context(|| format!("failed to parse spec: {}", path.display()))?;

    if !options.slice.is_noop() {
        let slice_report = slice::slice_openapi(&mut api, &options.slice)?;
        reports.extend(slice_report.report_entries());
    }

    Ok(LoadedOperationListingSpec { api, reports })
}

struct PreparedOpenApi {
    api: OpenAPI,
    reports: Vec<ReportEntry>,
    transform_plan: transform::TransformPlan,
}

fn prepare_openapi(path: &Path, options: &LoadOptions) -> Result<PreparedOpenApi> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read spec: {}", path.display()))?;
    let mut reports = Vec::new();
    let audits = Vec::new();
    let mut api = parse_prepared(&raw)
        .with_context(|| format!("failed to parse spec: {}", path.display()))?;

    if !options.slice.is_noop() {
        let slice_report = slice::slice_openapi(&mut api, &options.slice)?;
        reports.extend(slice_report.report_entries());
    }

    let transform_plan = transform::TransformPlan::from_reports_with_audits(&reports, audits);

    Ok(PreparedOpenApi {
        api,
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

fn inspect_openapi(
    spec: &OpenAPI,
    auth_policy: &AuthSelectionPolicy,
) -> Result<(SpecFacts, AuthPlan)> {
    let title = spec.info.title.clone();
    let bin_name = bin_name_from_title(&title);

    let (base_url, base_url_is_relative) = match spec.servers.first() {
        None => (None, false),
        Some(s) => {
            let is_relative = !(s.url.starts_with("http://") || s.url.starts_with("https://"));
            (Some(s.url.clone()), is_relative)
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
fn parse(raw: &str, _path: &Path) -> Result<(OpenAPI, Vec<ReportEntry>)> {
    Ok((parse_prepared(raw)?, Vec::new()))
}

fn parse_prepared(raw: &str) -> Result<OpenAPI> {
    // Try JSON first when the content looks like JSON for clearer parser errors.
    // Otherwise use YAML, which also accepts JSON-like syntax.
    let trimmed = raw.trim_start();
    if trimmed.starts_with('{') {
        serde_json::from_str(raw).map_err(|e| anyhow!("JSON parse error: {e}"))
    } else {
        serde_yaml::from_str(raw).map_err(|e| anyhow!("YAML parse error: {e}"))
    }
}

fn count_operations(spec: &OpenAPI) -> usize {
    traversal::operations(spec).len()
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
        let facts: SpecFacts = serde_yaml::from_str::<OpenAPI>(PETSTORE_MINIMAL)
            .map(|spec| {
                // exercise the same derivations inspect() uses
                SpecFacts {
                    title: spec.info.title.clone(),
                    bin_name: bin_name_from_title(&spec.info.title),
                    base_url: spec.servers.first().map(|s| s.url.clone()),
                    base_url_is_relative: false,
                    operation_count: count_operations(&spec),
                    auth_kind: auth::derive_auth_kind(&spec).unwrap(),
                }
            })
            .unwrap();
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
