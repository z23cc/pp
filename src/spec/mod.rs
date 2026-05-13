//! OpenAPI spec inspection: parse a 3.0 spec and derive the facts pp needs
//! to drive progenitor + wrapper templates.

pub mod normalize;

use anyhow::{anyhow, Context, Result};
use heck::ToKebabCase;
use openapiv3::{OpenAPI, ReferenceOr, SecurityScheme};
use regex::Regex;
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

type DowngradeReport = Option<(String, usize)>;

/// Parse the spec at `path` (YAML or JSON, detected by extension and content),
/// normalize it for progenitor, and derive [`SpecFacts`].
pub fn load(path: &Path) -> Result<LoadedSpec> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read spec: {}", path.display()))?;
    let mut spec =
        parse(&raw, path).with_context(|| format!("failed to parse spec: {}", path.display()))?;
    let normalization_warnings = normalize::normalize(&mut spec)?;
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

fn parse(raw: &str, _path: &Path) -> Result<OpenAPI> {
    let (owned, downgraded) = downgrade_openapi_31(raw)?;
    let parse_raw = owned.as_deref().unwrap_or(raw);

    if let Some((version, transforms)) = downgraded {
        eprintln!(
            "pp: downgraded OpenAPI {version} → 3.0.3 for parsing ({transforms} transforms applied)"
        );
    }

    // Try JSON first if it looks like JSON, otherwise YAML. serde_yaml accepts
    // JSON too, so YAML is a safe fallback.
    let trimmed = parse_raw.trim_start();
    if trimmed.starts_with('{') {
        serde_json::from_str(parse_raw).map_err(|e| anyhow!("JSON parse error: {e}"))
    } else {
        serde_yaml::from_str(parse_raw).map_err(|e| anyhow!("YAML parse error: {e}"))
    }
}

fn downgrade_openapi_31(raw: &str) -> Result<(Option<String>, DowngradeReport)> {
    let Some(version) = detect_openapi_31(raw) else {
        return Ok((None, None));
    };

    let mut transforms = 0;
    let mut out = raw.to_string();

    out = replace_count(
        &out,
        &Regex::new(r#"(?m)^(\s*openapi:\s*)['\"]?3\.1(?:\.\d+)?['\"]?\s*$"#)?,
        "${1}3.0.3",
        &mut transforms,
    );
    out = replace_count(
        &out,
        &Regex::new(r#"\"openapi\"\s*:\s*\"3\.1(?:\.\d+)?\""#)?,
        r#""openapi":"3.0.3""#,
        &mut transforms,
    );
    out = replace_count(
        &out,
        &Regex::new(
            r#"(?m)^(\s*)type:\s*\[\s*['\"]?(string|integer|number|boolean|array|object)['\"]?\s*,\s*['\"]?null['\"]?\s*\]\s*$"#,
        )?,
        "${1}type: $2\n${1}nullable: true",
        &mut transforms,
    );
    out = replace_count(
        &out,
        &Regex::new(
            r#"(?m)^(\s*)type:\s*\[\s*['\"]?null['\"]?\s*,\s*['\"]?(string|integer|number|boolean|array|object)['\"]?\s*\]\s*$"#,
        )?,
        "${1}type: $2\n${1}nullable: true",
        &mut transforms,
    );
    out = replace_count(
        &out,
        &Regex::new(
            r#"(?m)^(\s*)type:\s*\n\s*-\s*['\"]?(string|integer|number|boolean|array|object)['\"]?\s*\n\s*-\s*['\"]?null['\"]?\s*$"#,
        )?,
        "${1}type: $2\n${1}nullable: true",
        &mut transforms,
    );
    out = replace_count(
        &out,
        &Regex::new(
            r#"(?m)^(\s*)type:\s*\n\s*-\s*['\"]?null['\"]?\s*\n\s*-\s*['\"]?(string|integer|number|boolean|array|object)['\"]?\s*$"#,
        )?,
        "${1}type: $2\n${1}nullable: true",
        &mut transforms,
    );
    out = replace_count(
        &out,
        &Regex::new(r#"\"exclusiveMinimum\"\s*:\s*(-?\d+(?:\.\d+)?)"#)?,
        r#""exclusiveMinimum": true, "minimum": $1"#,
        &mut transforms,
    );
    out = replace_count(
        &out,
        &Regex::new(r#"\"exclusiveMaximum\"\s*:\s*(-?\d+(?:\.\d+)?)"#)?,
        r#""exclusiveMaximum": true, "maximum": $1"#,
        &mut transforms,
    );
    out = strip_top_level_block(&out, "webhooks", &mut transforms);
    out = strip_top_level_block(&out, "$defs", &mut transforms);

    Ok((Some(out), Some((version, transforms))))
}

fn replace_count(input: &str, re: &Regex, replacement: &str, transforms: &mut usize) -> String {
    let count = re.find_iter(input).count();
    if count > 0 {
        *transforms += count;
        re.replace_all(input, replacement).into_owned()
    } else {
        input.to_string()
    }
}

fn strip_top_level_block(input: &str, key: &str, transforms: &mut usize) -> String {
    let mut out = Vec::new();
    let mut skipping = false;
    let header = format!("{key}:");

    for line in input.lines() {
        let is_top_level = !line.starts_with(char::is_whitespace);
        if is_top_level && line.trim_end() == header {
            skipping = true;
            *transforms += 1;
            continue;
        }
        if skipping && is_top_level && !line.trim().is_empty() {
            skipping = false;
        }
        if !skipping {
            out.push(line);
        }
    }

    let mut joined = out.join("\n");
    if input.ends_with('\n') {
        joined.push('\n');
    }
    joined
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
    let after_key = compact
        .split_once("\"openapi\":\"")
        .map(|(_, value)| value)?;
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
            AuthKind::Bearer | AuthKind::HttpBasic | AuthKind::ApiKey { .. } => {
                return Ok(auth_kind)
            }
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

    Ok(first_unsupported.unwrap_or(AuthKind::None))
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
            detect_openapi_31(r#"{"openapi":"3.1.1","paths":{}}"#).as_deref(),
            Some("3.1.1")
        );
    }

    #[test]
    fn openapi_31_yaml_downgrades_nullable_type_before_parse() {
        let spec = parse(
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
}
