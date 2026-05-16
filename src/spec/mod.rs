//! OpenAPI spec inspection: parse a 3.0 spec and derive the facts pp needs
//! to drive progenitor + wrapper templates.

mod auth;
pub(crate) use auth::{AuthPlan, AuthSelectionPolicy};
pub(crate) mod normalization_rules;
pub mod normalize;
mod pre_parse;
pub(crate) mod references;
pub mod report;
pub mod slice;
pub(crate) mod transform;
pub(crate) mod traversal;

use crate::backend::BackendCapabilities;
use anyhow::{anyhow, Context, Result};
use heck::ToKebabCase;
use openapiv3::OpenAPI;
use regex::Regex;
use report::{ReportEntry, ReportStage};
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

/// Everything pp extracts from a spec before invoking progenitor + templates.
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
    pub normalization_warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct LoadOptions {
    pub slice: slice::SliceOptions,
    pub policy: transform::TransformPolicy,
    pub auth_policy: AuthSelectionPolicy,
    pub backend_capabilities: BackendCapabilities,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            slice: slice::SliceOptions::default(),
            policy: transform::TransformPolicy::default(),
            auth_policy: AuthSelectionPolicy::default(),
            backend_capabilities: BackendCapabilities::progenitor(),
        }
    }
}

impl LoadOptions {
    pub(crate) fn with_backend_capabilities(
        mut self,
        backend_capabilities: BackendCapabilities,
    ) -> Self {
        self.backend_capabilities = backend_capabilities;
        self
    }
}

/// Parse the spec at `path` (YAML or JSON, detected by extension and content),
/// normalize it for progenitor, and derive [`SpecFacts`].
pub(crate) fn load(path: &Path) -> Result<LoadedSpec> {
    load_with_options(path, &LoadOptions::default())
}

pub(crate) fn load_with_options(path: &Path, options: &LoadOptions) -> Result<LoadedSpec> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read spec: {}", path.display()))?;
    let raw_repair_plan = pre_parse::RawSpecRepairPlan::propose(&raw)?;
    let raw_reports = raw_repair_plan.report_entries();
    let mut audits = raw_repair_plan.audit_entries();
    let mut proposed_raw_plan = transform::TransformPlan::from_reports(&raw_reports);
    proposed_raw_plan.approve(&options.policy)?;
    let repaired_raw = if raw_repair_plan.is_empty() {
        None
    } else {
        Some(raw_repair_plan.apply(&raw)?)
    };
    let parse_raw = repaired_raw.as_deref().unwrap_or(&raw);
    let mut spec = parse_prepared(parse_raw)
        .with_context(|| format!("failed to parse spec: {}", path.display()))?;
    let mut reports = raw_reports;
    if !options.slice.is_noop() {
        let slice_report = slice::slice_openapi(&mut spec, &options.slice)?;
        reports.extend(slice_report.report_entries());
    }
    let approved_typed_normalization_transforms =
        normalize::propose_typed_normalization_transforms(&spec, &options.backend_capabilities);
    let proposed_reports = approved_typed_normalization_transforms.report_entries();
    let typed_audits = approved_typed_normalization_transforms.audit_entries();
    let mut proposed_plan = transform::TransformPlan::from_reports(&proposed_reports);
    proposed_plan.approve(&options.policy)?;

    reports.extend(
        normalize::normalize_with_approved_typed_normalization_transforms(
            &mut spec,
            &options.backend_capabilities,
            &approved_typed_normalization_transforms,
        )?,
    );
    audits.extend(typed_audits);
    let mut transform_plan = transform::TransformPlan::from_reports_with_audits(&reports, audits);
    transform_plan.approve(&options.policy)?;
    let (facts, auth_plan) = inspect_openapi(&spec, &options.auth_policy)?;
    let normalization_reports = reports
        .iter()
        .filter(|report| report.stage != ReportStage::PreParseTolerance)
        .cloned()
        .collect::<Vec<_>>();
    let normalization_warnings = report::formatted_warnings(&normalization_reports);

    Ok(LoadedSpec {
        api: spec,
        facts,
        auth_plan,
        reports,
        transform_plan,
        normalization_warnings,
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
    let (owned, reports) = pre_parse::normalize_yaml(raw)?;
    let parse_raw = owned.as_deref().unwrap_or(raw);
    Ok((parse_prepared(parse_raw)?, reports))
}

fn parse_prepared(raw: &str) -> Result<OpenAPI> {
    // Try JSON first if it looks like JSON, otherwise YAML. serde_yaml accepts
    // JSON too, so YAML is a safe fallback.
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
    use openapiv3::ReferenceOr;

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
    fn openapi_31_json_is_detected() {
        assert_eq!(
            pre_parse::detect_openapi_31(r#"{"openapi":"3.1.1","paths":{}}"#).as_deref(),
            Some("3.1.1")
        );
    }

    #[test]
    fn openapi_31_yaml_downgrades_nullable_type_before_parse() {
        let (spec, reports) = parse(
            r#"
openapi: 3.1.0
info:
  title: Future API
  version: "1.0.0"
paths: {}
components:
  schemas:
    MaybeName:
      type: [string, null]
"#,
            Path::new("future.yaml"),
        )
        .unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].stage, ReportStage::PreParseTolerance);
        assert_eq!(reports[0].code, "spec.pre_parse.openapi_31_downgraded");
        assert_eq!(
            reports[0].formatted_warning(),
            "downgraded OpenAPI 3.1.0 → 3.0.3 for parsing (2 transforms applied)"
        );

        let components = spec.components.unwrap();
        let ReferenceOr::Item(schema) = components.schemas.get("MaybeName").unwrap() else {
            panic!("expected inline schema");
        };
        assert!(schema.schema_data.nullable);
        assert!(matches!(
            schema.schema_kind,
            openapiv3::SchemaKind::Type(openapiv3::Type::String(_))
        ));
    }

    #[test]
    fn pre_parse_reports_remain_outside_typed_normalization_proposals() {
        let (spec, pre_parse_reports) = parse(
            r#"
openapi: 3.1.0
info:
  title: Future API
  version: "1.0.0"
paths: {}
components:
  schemas:
    MaybeName:
      type: [string, null]
"#,
            Path::new("future.yaml"),
        )
        .unwrap();
        assert!(pre_parse_reports
            .iter()
            .any(|report| report.code == "spec.pre_parse.openapi_31_downgraded"));

        let typed_plan = normalize::propose_typed_normalization_transforms(
            &spec,
            &BackendCapabilities::progenitor(),
        );
        assert!(typed_plan
            .report_entries()
            .iter()
            .all(|report| report.stage == ReportStage::TypedNormalization));
        assert!(!typed_plan
            .report_entries()
            .iter()
            .any(|report| report.code.starts_with("spec.pre_parse.")));
    }

    #[test]
    fn raw_repair_plan_proposes_reports_and_applies_after_approval() {
        let raw = r#"
openapi: 3.1.0
info:
  title: Raw Plan Fixture
  version: "1.0.0"
tags:
  - name: account
    description:
      text: Accounts
paths:
  /things/{thing_id}:
    get:
      $ref: "resources/things/list.yml"
components:
  schemas:
    MaybeName:
      type: [string, null]
      maximum: 9223372036854776008
"#;
        let plan = pre_parse::RawSpecRepairPlan::propose(raw).unwrap();
        let reports = plan.report_entries();

        assert_eq!(reports.len(), 4);
        assert_eq!(
            reports.iter().map(|report| report.code).collect::<Vec<_>>(),
            vec![
                "spec.pre_parse.openapi_31_downgraded",
                "spec.pre_parse.numeric_bounds_clamped",
                "spec.pre_parse.tag_descriptions_replaced",
                "spec.pre_parse.ref_only_operations_replaced",
            ]
        );
        assert!(raw.contains("openapi: 3.1.0"));
        assert!(raw.contains("$ref: \"resources/things/list.yml\""));

        let repaired = plan.apply(raw).unwrap();
        assert!(repaired.contains("openapi: 3.0.3"));
        assert!(repaired.contains("nullable: true"));
        assert!(repaired.contains("maximum: 9223372036854775807"));
        assert!(repaired.contains("    description: \"\""));
        assert!(repaired.contains("operationId: getresources_things_list_yml"));
    }

    #[test]
    fn transform_plan_carries_raw_and_typed_audit_entries() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec_path = temp.path().join("audit.yaml");
        std::fs::write(
            &spec_path,
            r#"
openapi: 3.1.0
info:
  title: Audit Fixture
  version: "1.0.0"
servers:
  - url: https://example.test
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
        '404':
          description: missing
"#,
        )
        .unwrap();

        let loaded = load_with_options(
            &spec_path,
            &LoadOptions {
                policy: transform::TransformPolicy::compatibility(),
                ..LoadOptions::default()
            },
        )
        .unwrap();

        let plan_json = serde_json::to_value(&loaded.transform_plan).unwrap();
        let entries = plan_json["entries"].as_array().expect("entries array");
        assert!(entries
            .iter()
            .any(|entry| entry["code"] == "spec.pre_parse.openapi_31_downgraded"));
        assert!(entries
            .iter()
            .any(|entry| entry["code"] == "spec.normalize.response_variants_pruned"));

        let audits = plan_json["audits"].as_array().expect("audits array");
        assert!(audits.iter().any(|audit| {
            audit["source_stage"] == "pre_parse_tolerance"
                && audit["code"] == "spec.pre_parse.openapi_31_downgraded"
                && audit["target"] == "raw.openapi.version"
                && audit.get("before").is_some()
                && audit.get("after").is_some()
        }));
        assert!(audits.iter().any(|audit| {
            audit["source_stage"] == "typed_normalization"
                && audit["code"] == "spec.normalize.response_variants_pruned"
                && audit["target"] == "operation GET /pets responses"
                && audit["target_pointer"] == "/paths/~1pets/get/responses"
                && audit["action_kind"] == "prune"
                && audit["backend_requirement_id"] == "progenitor.response.single_variant"
                && audit.get("backend_requirement").is_some()
                && audit.get("before_json").is_some()
                && audit.get("after_json").is_some()
        }));
    }

    #[test]
    fn raw_pre_parse_reports_are_policy_checked_before_parse() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec_path = temp.path().join("future.yaml");
        std::fs::write(
            &spec_path,
            r#"
openapi: 3.1.0
info:
  title: Future API
  version: "1.0.0"
paths: {}
"#,
        )
        .unwrap();

        let strict_err = match load_with_options(&spec_path, &LoadOptions::default()) {
            Ok(_) => panic!("strict load unexpectedly succeeded"),
            Err(err) => err,
        };
        let strict_message = strict_err.to_string();
        assert!(strict_message.contains("strict transform policy rejected"));
        assert!(strict_message.contains("spec.pre_parse.openapi_31_downgraded"));

        let options = LoadOptions {
            policy: transform::TransformPolicy::strict()
                .allow_code("spec.pre_parse.openapi_31_downgraded"),
            ..LoadOptions::default()
        };
        let loaded = load_with_options(&spec_path, &options).unwrap();
        assert!(loaded
            .reports
            .iter()
            .any(|report| report.code == "spec.pre_parse.openapi_31_downgraded"));
    }

    #[test]
    fn out_of_range_numeric_bounds_are_clamped_before_parse() {
        let (out, count) = pre_parse::clamp_numeric_bounds(
            r#"
minimum: -9223372036854776000
maximum: 9223372036854776008
exclusiveMinimum: -9223372036854775808
{"maximum":9223372036854776008}
"#,
        )
        .unwrap();

        assert_eq!(count, 3);
        assert!(out.contains("minimum: -9223372036854775808"));
        assert!(out.contains("maximum: 9223372036854775807"));
        assert!(out.contains(r#"{"maximum":9223372036854775807}"#));
        assert!(out.contains("exclusiveMinimum: -9223372036854775808"));
    }

    #[test]
    fn top_level_tag_map_descriptions_are_replaced() {
        let (out, count) = pre_parse::normalize_top_level_tag_descriptions(
            r#"tags:
  - name: account
    description:
      text: Accounts
      format: markdown
paths: {}
"#,
        );

        assert_eq!(count, 1);
        assert!(out.contains("    description: \"\""));
        assert!(!out.contains("text: Accounts"));
        assert!(out.contains("paths: {}"));
    }

    #[test]
    fn ref_only_operations_get_parseable_placeholders() {
        let (out, count) = pre_parse::replace_ref_only_operations(
            r#"paths:
  /v2/things/{thing_id}:
    get:
      $ref: "resources/things/list.yml"
"#,
        )
        .unwrap();

        assert_eq!(count, 1);
        assert!(out.contains("operationId: getresources_things_list_yml"));
        assert!(out.contains("name: thing_id"));
        assert!(out.contains("responses:"));
    }
}
