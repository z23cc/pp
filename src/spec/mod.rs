//! OpenAPI spec inspection: parse a 3.0 spec and derive the facts pp needs
//! to drive progenitor + wrapper templates.

pub(crate) mod normalization_rules;
pub mod normalize;
mod pre_parse;
pub(crate) mod references;
pub mod report;
pub mod slice;
pub(crate) mod traversal;

use anyhow::{anyhow, Context, Result};
use heck::ToKebabCase;
use openapiv3::{OpenAPI, Parameter, ReferenceOr, SecurityScheme};
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

pub struct LoadedSpec {
    pub api: OpenAPI,
    pub facts: SpecFacts,
    pub reports: Vec<ReportEntry>,
    pub normalization_warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    pub slice: slice::SliceOptions,
}

/// Parse the spec at `path` (YAML or JSON, detected by extension and content),
/// normalize it for progenitor, and derive [`SpecFacts`].
pub fn load(path: &Path) -> Result<LoadedSpec> {
    load_with_options(path, &LoadOptions::default())
}

pub fn load_with_options(path: &Path, options: &LoadOptions) -> Result<LoadedSpec> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read spec: {}", path.display()))?;
    let (mut spec, mut reports) =
        parse(&raw, path).with_context(|| format!("failed to parse spec: {}", path.display()))?;
    reports.extend(normalize::normalize(&mut spec)?);
    if !options.slice.is_noop() {
        let slice_report = slice::slice_openapi(&mut spec, &options.slice)?;
        reports.extend(slice_report.report_entries());
    }
    let facts = inspect_openapi(&spec)?;
    let normalization_reports = reports
        .iter()
        .filter(|report| report.stage != ReportStage::PreParseTolerance)
        .cloned()
        .collect::<Vec<_>>();
    let normalization_warnings = report::formatted_warnings(&normalization_reports);

    Ok(LoadedSpec {
        api: spec,
        facts,
        reports,
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
pub fn inspect_with_options(path: &Path, options: &LoadOptions) -> Result<SpecFacts> {
    Ok(load_with_options(path, options)?.facts)
}

fn inspect_openapi(spec: &OpenAPI) -> Result<SpecFacts> {
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
    let auth_kind = derive_auth_kind(spec)?;

    Ok(SpecFacts {
        title,
        bin_name,
        base_url,
        base_url_is_relative,
        operation_count,
        auth_kind,
    })
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

fn parse(raw: &str, _path: &Path) -> Result<(OpenAPI, Vec<ReportEntry>)> {
    let (owned, reports) = pre_parse::normalize_yaml(raw)?;
    let parse_raw = owned.as_deref().unwrap_or(raw);

    // Try JSON first if it looks like JSON, otherwise YAML. serde_yaml accepts
    // JSON too, so YAML is a safe fallback.
    let trimmed = parse_raw.trim_start();
    let spec = if trimmed.starts_with('{') {
        serde_json::from_str(parse_raw).map_err(|e| anyhow!("JSON parse error: {e}"))?
    } else {
        serde_yaml::from_str(parse_raw).map_err(|e| anyhow!("YAML parse error: {e}"))?
    };
    Ok((spec, reports))
}

fn count_operations(spec: &OpenAPI) -> usize {
    traversal::operations(spec).len()
}

fn derive_auth_kind(spec: &OpenAPI) -> Result<AuthKind> {
    let Some(components) = &spec.components else {
        return Ok(derive_query_api_key_auth(spec).unwrap_or(AuthKind::None));
    };
    if components.security_schemes.is_empty() {
        return Ok(derive_query_api_key_auth(spec).unwrap_or(AuthKind::None));
    }

    let mut first_unsupported = None;
    let mut oauth2_bearer = false;
    for (_name, scheme_ref) in &components.security_schemes {
        let auth_kind = match scheme_ref {
            ReferenceOr::Item(SecurityScheme::OAuth2 { .. }) => {
                // User supplies their own token via `<BIN>_TOKEN`; pp doesn't run the OAuth2 flow.
                oauth2_bearer = true;
                AuthKind::None
            }
            ReferenceOr::Item(scheme) => auth_kind_for_scheme(scheme),
            ReferenceOr::Reference { reference } => AuthKind::Unsupported {
                reason: format!("$ref security scheme not supported in MVP: {reference}"),
            },
        };

        match auth_kind {
            AuthKind::Bearer
            | AuthKind::HttpBasic
            | AuthKind::ApiKey { .. }
            | AuthKind::QueryApiKey { .. } => return Ok(auth_kind),
            AuthKind::Unsupported { .. } => {
                if first_unsupported.is_none() {
                    first_unsupported = Some(auth_kind);
                }
            }
            AuthKind::None => {}
        }
    }

    if oauth2_bearer {
        return Ok(AuthKind::Bearer);
    }

    Ok(first_unsupported
        .or_else(|| derive_query_api_key_auth(spec))
        .unwrap_or(AuthKind::None))
}

fn derive_query_api_key_auth(spec: &OpenAPI) -> Option<AuthKind> {
    let operations = traversal::operations(spec);

    if operations.is_empty() {
        return None;
    }

    let mut candidates: indexmap::IndexMap<String, QueryAuthStats> = indexmap::IndexMap::new();
    let mut first_required_query_names = Vec::new();

    for operation_ref in operations {
        let required_query_params = required_query_params(
            operation_ref
                .path_parameters
                .iter()
                .chain(operation_ref.operation.parameters.iter()),
        );
        let Some(first_param) = required_query_params.first() else {
            first_required_query_names.push(None);
            continue;
        };
        first_required_query_names.push(Some(first_param.name.clone()));

        for param in required_query_params {
            if !is_auth_query_param_name(&param.name) {
                continue;
            }
            let key = param.name.to_ascii_lowercase();
            let stats = candidates.entry(key).or_insert_with(|| QueryAuthStats {
                param_name: param.name.clone(),
                appearances: 0,
            });
            stats.appearances += 1;
        }
    }

    let operation_count = first_required_query_names.len();
    for stats in candidates.values() {
        let appears_in_half = stats.appearances * 2 >= operation_count;
        let first_in_every_operation = first_required_query_names
            .iter()
            .all(|name| name.as_deref() == Some(stats.param_name.as_str()));
        if appears_in_half || first_in_every_operation {
            return Some(AuthKind::QueryApiKey {
                param_name: stats.param_name.clone(),
            });
        }
    }

    None
}

#[derive(Debug)]
struct QueryAuthStats {
    param_name: String,
    appearances: usize,
}

fn required_query_params<'a>(
    params: impl Iterator<Item = &'a ReferenceOr<Parameter>>,
) -> Vec<&'a openapiv3::ParameterData> {
    params
        .filter_map(|param| match param {
            ReferenceOr::Item(Parameter::Query { parameter_data, .. })
                if parameter_data.required =>
            {
                Some(parameter_data)
            }
            _ => None,
        })
        .collect()
}

fn is_auth_query_param_name(name: &str) -> bool {
    let normalized: String = name
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        normalized.as_str(),
        "apikey"
            | "accesstoken"
            | "token"
            | "appid"
            | "appkey"
            | "license"
            | "authkey"
            | "subscriptionkey"
    )
}

fn auth_kind_for_scheme(scheme: &SecurityScheme) -> AuthKind {
    match scheme {
        SecurityScheme::HTTP { scheme, .. } if scheme.eq_ignore_ascii_case("bearer") => {
            AuthKind::Bearer
        }
        SecurityScheme::HTTP { scheme, .. } if scheme.eq_ignore_ascii_case("basic") => {
            AuthKind::HttpBasic
        }
        SecurityScheme::HTTP { scheme, .. } => AuthKind::Unsupported {
            reason: format!("http auth scheme '{scheme}' not supported in MVP (only bearer/basic)"),
        },
        SecurityScheme::APIKey { location, name, .. } => match location {
            openapiv3::APIKeyLocation::Header => AuthKind::ApiKey {
                header_name: name.clone(),
            },
            other => AuthKind::Unsupported {
                reason: format!("apiKey in '{other:?}' not supported in MVP (only header)"),
            },
        },
        SecurityScheme::OAuth2 { .. } => AuthKind::Bearer,
        SecurityScheme::OpenIDConnect { .. } => AuthKind::Unsupported {
            reason: "OpenID Connect not supported in MVP".into(),
        },
    }
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

    const BEARER_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: My API
  version: "1.0.0"
paths: {}
components:
  securitySchemes:
    bearerAuth:
      type: http
      scheme: bearer
"#;

    const APIKEY_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: My API
  version: "1.0.0"
paths: {}
components:
  securitySchemes:
    apiKeyAuth:
      type: apiKey
      in: header
      name: X-API-Key
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
                    auth_kind: derive_auth_kind(&spec).unwrap(),
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

    #[test]
    fn bearer_auth_detected() {
        let spec: OpenAPI = serde_yaml::from_str(BEARER_SPEC).unwrap();
        assert_eq!(derive_auth_kind(&spec).unwrap(), AuthKind::Bearer);
    }

    #[test]
    fn http_basic_auth_detected() {
        let spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: My API
  version: "1.0.0"
paths: {}
components:
  securitySchemes:
    basicAuth:
      type: http
      scheme: basic
"#,
        )
        .unwrap();

        assert_eq!(derive_auth_kind(&spec).unwrap(), AuthKind::HttpBasic);
    }

    #[test]
    fn oauth2_first_bearer_second_detects_bearer() {
        let spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: My API
  version: "1.0.0"
paths: {}
components:
  securitySchemes:
    oauth2:
      type: oauth2
      flows: {}
    bearerAuth:
      type: http
      scheme: bearer
"#,
        )
        .unwrap();

        assert_eq!(derive_auth_kind(&spec).unwrap(), AuthKind::Bearer);
    }

    #[test]
    fn oauth2_only_detects_bearer() {
        let spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: My API
  version: "1.0.0"
paths: {}
components:
  securitySchemes:
    oauth2:
      type: oauth2
      flows: {}
"#,
        )
        .unwrap();

        assert_eq!(derive_auth_kind(&spec).unwrap(), AuthKind::Bearer);
    }

    #[test]
    fn all_unsupported_auth_returns_first_unsupported() {
        let spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: My API
  version: "1.0.0"
paths: {}
components:
  securitySchemes:
    digestAuth:
      type: http
      scheme: digest
    openId:
      type: openIdConnect
      openIdConnectUrl: https://example.com/.well-known/openid-configuration
"#,
        )
        .unwrap();

        assert_eq!(
            derive_auth_kind(&spec).unwrap(),
            AuthKind::Unsupported {
                reason: "http auth scheme 'digest' not supported in MVP (only bearer/basic)".into()
            }
        );
    }

    #[test]
    fn apikey_header_detected() {
        let spec: OpenAPI = serde_yaml::from_str(APIKEY_SPEC).unwrap();
        assert_eq!(
            derive_auth_kind(&spec).unwrap(),
            AuthKind::ApiKey {
                header_name: "X-API-Key".into()
            }
        );
    }

    #[test]
    fn required_license_query_param_in_all_ops_detects_query_api_key() {
        let spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Weather API
  version: "1.0.0"
paths:
  /weather:
    get:
      parameters:
        - in: query
          name: license
          required: true
          schema:
            type: string
        - in: query
          name: city
          required: true
          schema:
            type: string
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();

        assert_eq!(
            derive_auth_kind(&spec).unwrap(),
            AuthKind::QueryApiKey {
                param_name: "license".into()
            }
        );
    }

    #[test]
    fn path_level_query_api_key_detects_query_api_key() {
        let spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Path Level Auth API
  version: "1.0.0"
paths:
  /weather:
    parameters:
      - in: query
        name: api_key
        required: true
        schema:
          type: string
    get:
      responses:
        '200':
          description: ok
    post:
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();

        assert_eq!(
            derive_auth_kind(&spec).unwrap(),
            AuthKind::QueryApiKey {
                param_name: "api_key".into()
            }
        );
    }

    #[test]
    fn inconsistent_auth_like_query_param_is_not_auth() {
        let spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Mixed API
  version: "1.0.0"
paths:
  /one:
    get:
      parameters:
        - in: query
          name: token
          required: true
          schema:
            type: string
      responses:
        '200':
          description: ok
  /two:
    get:
      parameters:
        - in: query
          name: account
          required: true
          schema:
            type: string
      responses:
        '200':
          description: ok
  /three:
    get:
      parameters:
        - in: query
          name: region
          required: true
          schema:
            type: string
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();

        assert_eq!(derive_auth_kind(&spec).unwrap(), AuthKind::None);
    }
}
