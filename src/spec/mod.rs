//! OpenAPI spec inspection: parse a 3.0 spec and derive the facts pp needs
//! to drive progenitor + wrapper templates.

pub mod normalize;

use anyhow::{anyhow, Context, Result};
use heck::ToKebabCase;
use openapiv3::{OpenAPI, ReferenceOr, SecurityScheme};
use serde::Serialize;
use std::path::Path;

/// Auth shape pp can template a wrapper for. Anything outside this set is
/// MVP-unsupported and surfaces as `AuthKind::Unsupported { reason }`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthKind {
    None,
    Bearer,
    ApiKey { header_name: String },
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
    pub normalization_warnings: Vec<String>,
}

/// Parse the spec at `path` (YAML or JSON, detected by extension and content),
/// normalize it for progenitor, and derive [`SpecFacts`].
pub fn load(path: &Path) -> Result<LoadedSpec> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read spec: {}", path.display()))?;
    let mut spec =
        parse(&raw, path).with_context(|| format!("failed to parse spec: {}", path.display()))?;
    let normalization_warnings = normalize::normalize(&mut spec);
    let facts = inspect_openapi(&spec)?;

    Ok(LoadedSpec {
        api: spec,
        facts,
        normalization_warnings,
    })
}

/// Parse the spec at `path` (YAML or JSON, detected by extension and content)
/// and derive [`SpecFacts`].
pub fn inspect(path: &Path) -> Result<SpecFacts> {
    Ok(load(path)?.facts)
}

fn inspect_openapi(spec: &OpenAPI) -> Result<SpecFacts> {
    let title = spec.info.title.clone();
    let bin_name = title.to_kebab_case();

    let (base_url, base_url_is_relative) = match spec.servers.first() {
        None => (None, false),
        Some(s) => {
            let is_relative = !(s.url.starts_with("http://") || s.url.starts_with("https://"));
            (Some(s.url.clone()), is_relative)
        }
    };

    let operation_count = count_operations(&spec);
    let auth_kind = derive_auth_kind(&spec)?;

    Ok(SpecFacts {
        title,
        bin_name,
        base_url,
        base_url_is_relative,
        operation_count,
        auth_kind,
    })
}

fn parse(raw: &str, path: &Path) -> Result<OpenAPI> {
    if let Some(version) = detect_openapi_31(raw) {
        return Err(anyhow!(
            "OpenAPI 3.1 is not yet supported by pp (uses openapiv3 crate which targets 3.0). Found 'openapi: {version}' in {}. See plan: docs/plans/2026-05-12-001-feat-rust-printing-press-mvp-plan.md scope boundaries.",
            path.display()
        ));
    }

    // Try JSON first if it looks like JSON, otherwise YAML. serde_yaml accepts
    // JSON too, so YAML is a safe fallback.
    let trimmed = raw.trim_start();
    if trimmed.starts_with('{') {
        serde_json::from_str(raw).map_err(|e| anyhow!("JSON parse error: {e}"))
    } else {
        serde_yaml::from_str(raw).map_err(|e| anyhow!("YAML parse error: {e}"))
    }
}

fn detect_openapi_31(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let line = line.trim_start();
        let Some(value) = line.strip_prefix("openapi:") else {
            continue;
        };
        let version = value.trim().trim_matches(['\'', '"']);
        if version.starts_with("3.1") {
            return Some(version.to_string());
        }
    }

    let compact: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    let Some(after_key) = compact.split_once("\"openapi\":\"").map(|(_, value)| value) else {
        return None;
    };
    let version = after_key.split('"').next().unwrap_or_default();
    version.starts_with("3.1").then(|| version.to_string())
}

fn count_operations(spec: &OpenAPI) -> usize {
    let mut n = 0;
    for (_, path_item) in spec.paths.iter() {
        if let ReferenceOr::Item(item) = path_item {
            for op in [
                &item.get,
                &item.put,
                &item.post,
                &item.delete,
                &item.options,
                &item.head,
                &item.patch,
                &item.trace,
            ] {
                if op.is_some() {
                    n += 1;
                }
            }
        }
    }
    n
}

fn derive_auth_kind(spec: &OpenAPI) -> Result<AuthKind> {
    let Some(components) = &spec.components else {
        return Ok(AuthKind::None);
    };
    if components.security_schemes.is_empty() {
        return Ok(AuthKind::None);
    }

    let mut first_unsupported = None;
    for (_name, scheme_ref) in &components.security_schemes {
        let auth_kind = match scheme_ref {
            ReferenceOr::Item(scheme) => auth_kind_for_scheme(scheme),
            ReferenceOr::Reference { reference } => AuthKind::Unsupported {
                reason: format!("$ref security scheme not supported in MVP: {reference}"),
            },
        };

        match auth_kind {
            AuthKind::Bearer | AuthKind::ApiKey { .. } => return Ok(auth_kind),
            AuthKind::Unsupported { .. } => {
                if first_unsupported.is_none() {
                    first_unsupported = Some(auth_kind);
                }
            }
            AuthKind::None => {}
        }
    }

    Ok(first_unsupported.unwrap_or(AuthKind::None))
}

fn auth_kind_for_scheme(scheme: &SecurityScheme) -> AuthKind {
    match scheme {
        SecurityScheme::HTTP { scheme, .. } if scheme.eq_ignore_ascii_case("bearer") => {
            AuthKind::Bearer
        }
        SecurityScheme::HTTP { scheme, .. } => AuthKind::Unsupported {
            reason: format!("http auth scheme '{scheme}' not supported in MVP (only bearer)"),
        },
        SecurityScheme::APIKey { location, name, .. } => match location {
            openapiv3::APIKeyLocation::Header => AuthKind::ApiKey {
                header_name: name.clone(),
            },
            other => AuthKind::Unsupported {
                reason: format!("apiKey in '{other:?}' not supported in MVP (only header)"),
            },
        },
        SecurityScheme::OAuth2 { .. } => AuthKind::Unsupported {
            reason: "OAuth2 not supported in MVP".into(),
        },
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
                    bin_name: spec.info.title.to_kebab_case(),
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
    fn openapi_31_yaml_errors_before_parse() {
        let err = parse(
            r#"
openapi: 3.1.0
info:
  title: Future API
  version: "1.0.0"
paths: {}
"#,
            Path::new("future.yaml"),
        )
        .unwrap_err();

        assert!(err.to_string().contains("3.1"));
    }

    #[test]
    fn bearer_auth_detected() {
        let spec: OpenAPI = serde_yaml::from_str(BEARER_SPEC).unwrap();
        assert_eq!(derive_auth_kind(&spec).unwrap(), AuthKind::Bearer);
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
    basicAuth:
      type: http
      scheme: basic
    oauth2:
      type: oauth2
      flows: {}
"#,
        )
        .unwrap();

        assert_eq!(
            derive_auth_kind(&spec).unwrap(),
            AuthKind::Unsupported {
                reason: "http auth scheme 'basic' not supported in MVP (only bearer)".into()
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
}
