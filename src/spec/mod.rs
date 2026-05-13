//! OpenAPI spec inspection: parse a 3.0 spec and derive the facts pp needs
//! to drive progenitor + wrapper templates.

pub mod normalize;

use anyhow::{anyhow, Context, Result};
use heck::ToKebabCase;
use openapiv3::{OpenAPI, Operation, Parameter, ReferenceOr, SecurityScheme};
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
    let (owned, warnings) = pre_normalize_yaml(raw)?;
    let parse_raw = owned.as_deref().unwrap_or(raw);

    for warning in warnings {
        eprintln!("pp: {warning}");
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

fn pre_normalize_yaml(raw: &str) -> Result<(Option<String>, Vec<String>)> {
    let (mut owned, downgraded) = downgrade_openapi_31(raw)?;
    let mut current = owned.as_deref().unwrap_or(raw).to_string();
    let mut changed = owned.is_some();
    let mut warnings = Vec::new();

    if let Some((version, transforms)) = downgraded {
        warnings.push(format!(
            "downgraded OpenAPI {version} → 3.0.3 for parsing ({transforms} transforms applied)"
        ));
    }

    let (clamped, clamp_count) = clamp_numeric_bounds(&current)?;
    if clamp_count > 0 {
        current = clamped;
        changed = true;
        warnings.push(format!("clamped {clamp_count} out-of-range numeric bounds"));
    }

    let (normalized_tags, tag_count) = normalize_top_level_tag_descriptions(&current);
    if tag_count > 0 {
        current = normalized_tags;
        changed = true;
        warnings.push(format!(
            "replaced {tag_count} non-string top-level tag descriptions"
        ));
    }

    let (inlined_refs, ref_count) = replace_ref_only_operations(&current)?;
    if ref_count > 0 {
        current = inlined_refs;
        changed = true;
        warnings.push(format!(
            "replaced {ref_count} ref-only operations with parseable placeholders"
        ));
    }

    if changed {
        owned = Some(current);
    }

    Ok((owned, warnings))
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

    join_lines(&out, input.ends_with('\n'))
}

fn clamp_numeric_bounds(input: &str) -> Result<(String, usize)> {
    let yaml_re = Regex::new(
        r#"(?m)^(\s*(?:minimum|maximum|exclusiveMinimum|exclusiveMaximum):\s*)(-?\d+)(\s*)$"#,
    )?;
    let json_re =
        Regex::new(r#"(\"(?:minimum|maximum|exclusiveMinimum|exclusiveMaximum)\"\s*:\s*)(-?\d+)"#)?;
    let mut count = 0;

    let out = yaml_re
        .replace_all(input, |caps: &regex::Captures<'_>| {
            clamp_replacement(
                &caps[1],
                &caps[2],
                caps.get(3).map_or("", |m| m.as_str()),
                &mut count,
            )
        })
        .into_owned();
    let out = json_re
        .replace_all(&out, |caps: &regex::Captures<'_>| {
            clamp_replacement(&caps[1], &caps[2], "", &mut count)
        })
        .into_owned();

    Ok((out, count))
}

fn clamp_replacement(prefix: &str, literal: &str, suffix: &str, count: &mut usize) -> String {
    if let Some(clamped) = clamped_i64_literal(literal) {
        *count += 1;
        format!("{prefix}{clamped}{suffix}")
    } else {
        format!("{prefix}{literal}{suffix}")
    }
}

fn clamped_i64_literal(literal: &str) -> Option<&'static str> {
    const I64_MAX: &str = "9223372036854775807";
    const I64_MIN_ABS: &str = "9223372036854775808";

    if let Some(rest) = literal.strip_prefix('-') {
        integer_exceeds(rest, I64_MIN_ABS).then_some("-9223372036854775808")
    } else {
        integer_exceeds(literal, I64_MAX).then_some("9223372036854775807")
    }
}

fn integer_exceeds(value: &str, max: &str) -> bool {
    let normalized = value.trim_start_matches('0');
    let normalized = if normalized.is_empty() {
        "0"
    } else {
        normalized
    };
    normalized.len() > max.len() || (normalized.len() == max.len() && normalized > max)
}

fn normalize_top_level_tag_descriptions(input: &str) -> (String, usize) {
    let lines: Vec<&str> = input.lines().collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut i = 0;
    let mut in_tags = false;
    let mut count = 0;

    while i < lines.len() {
        let line = lines[i];
        if !line.starts_with(char::is_whitespace) && !line.trim().is_empty() {
            in_tags = line.trim_end() == "tags:";
        }

        if in_tags && line.trim_end() == "    description:" && child_map_follows(&lines, i + 1) {
            out.push("    description: \"\"");
            count += 1;
            i += 1;
            while i < lines.len() && is_description_child(lines[i]) {
                i += 1;
            }
            continue;
        }

        out.push(line);
        i += 1;
    }

    (join_lines(&out, input.ends_with('\n')), count)
}

fn child_map_follows(lines: &[&str], start: usize) -> bool {
    for line in &lines[start..] {
        if line.trim().is_empty() {
            continue;
        }
        return is_description_child(line);
    }
    false
}

fn is_description_child(line: &str) -> bool {
    (line.starts_with("      ") && !line.starts_with("    - ")) || line.trim().is_empty()
}

fn replace_ref_only_operations(input: &str) -> Result<(String, usize)> {
    let method_re = Regex::new(r#"^    (get|put|post|delete|patch|options|head|trace):\s*$"#)?;
    let ref_re = Regex::new(r#"^      \$ref:\s*['\"]([^'\"]+)['\"]\s*$"#)?;
    let path_re = Regex::new(r#"^  ([^\s].*):\s*$"#)?;
    let path_param_re = Regex::new(r#"\{([^}/]+)\}"#)?;
    let lines: Vec<&str> = input.lines().collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut current_path = String::new();
    let mut count = 0;
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        if let Some(caps) = path_re.captures(line) {
            current_path = caps[1].trim_matches('"').to_string();
        }

        if let Some(method_caps) = method_re.captures(line) {
            if let Some(next) = lines.get(i + 1) {
                if let Some(ref_caps) = ref_re.captures(next) {
                    count += 1;
                    let method = &method_caps[1];
                    let operation_id = operation_id_from_ref(method, &ref_caps[1]);
                    out.push(format!("    {method}:"));
                    out.push(format!("      operationId: {operation_id}"));
                    let path_params: Vec<String> = path_param_re
                        .captures_iter(&current_path)
                        .map(|caps| caps[1].to_string())
                        .collect();
                    if !path_params.is_empty() {
                        out.push("      parameters:".to_string());
                        for param in path_params {
                            out.push("        - in: path".to_string());
                            out.push(format!("          name: {param}"));
                            out.push("          required: true".to_string());
                            out.push("          schema:".to_string());
                            out.push("            type: string".to_string());
                        }
                    }
                    out.push("      responses:".to_string());
                    out.push("        '200':".to_string());
                    out.push("          description: ok".to_string());
                    i += 2;
                    continue;
                }
            }
        }

        out.push(line.to_string());
        i += 1;
    }

    let out_refs: Vec<&str> = out.iter().map(String::as_str).collect();
    Ok((join_lines(&out_refs, input.ends_with('\n')), count))
}

fn operation_id_from_ref(method: &str, reference: &str) -> String {
    let mut out = String::from(method);
    for ch in reference.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

fn join_lines(lines: &[&str], trailing_newline: bool) -> String {
    let mut joined = lines.join("\n");
    if trailing_newline {
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
    let mut operations = Vec::new();
    for (_, path_item) in spec.paths.iter() {
        let ReferenceOr::Item(item) = path_item else {
            continue;
        };
        for operation in [
            &item.get,
            &item.put,
            &item.post,
            &item.delete,
            &item.options,
            &item.head,
            &item.patch,
            &item.trace,
        ]
        .into_iter()
        .flatten()
        {
            let mut params = item.parameters.clone();
            params.extend(operation.parameters.clone());
            operations.push((operation, params));
        }
    }

    if operations.is_empty() {
        return None;
    }

    let mut candidates: indexmap::IndexMap<String, QueryAuthStats> = indexmap::IndexMap::new();
    let mut first_required_query_names = Vec::new();

    for (operation, params) in operations {
        let required_query_params = required_query_params(operation, &params);
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
    _operation: &'a Operation,
    params: &'a [ReferenceOr<Parameter>],
) -> Vec<&'a openapiv3::ParameterData> {
    params
        .iter()
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
    fn out_of_range_numeric_bounds_are_clamped_before_parse() {
        let (out, count) = clamp_numeric_bounds(
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
        let (out, count) = normalize_top_level_tag_descriptions(
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
        let (out, count) = replace_ref_only_operations(
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
