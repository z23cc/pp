use anyhow::Result;
use openapiv3::{OpenAPI, Parameter, ReferenceOr, SecurityScheme};

use crate::spec::{traversal, AuthKind};

pub(super) fn derive_auth_kind(spec: &OpenAPI) -> Result<AuthKind> {
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
