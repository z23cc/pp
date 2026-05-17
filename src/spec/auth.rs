use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::Value;

use crate::spec::{traversal, AuthKind, PpParameterLocation, PpSpec};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum AuthSelectionPolicy {
    #[default]
    FailAmbiguous,
    ExplicitScheme {
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct AuthPlan {
    pub candidates: Vec<AuthCandidate>,
    pub requirements: AuthRequirementModel,
    pub decision: AuthDecision,
    pub selected: AuthKind,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct AuthRequirementModel {
    pub global: Vec<AuthRequirementAlternative>,
    pub operations_inheriting_global: usize,
    pub operation_overrides: Vec<AuthOperationRequirement>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct AuthRequirementAlternative {
    pub scheme_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct AuthOperationRequirement {
    pub method: String,
    pub path: String,
    pub operation_id: Option<String>,
    pub requirements: Vec<AuthRequirementAlternative>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AuthCandidate {
    SecurityScheme {
        name: String,
        auth_kind: AuthKind,
        supported: bool,
        reason: Option<String>,
    },
    QueryParameterHeuristic {
        param_name: String,
        appearances: usize,
        operation_count: usize,
        accepted: bool,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AuthDecision {
    None,
    Selected {
        source: AuthSelectionSource,
        selection_basis: AuthSelectionBasis,
    },
    AmbiguousSelected {
        selected_source: AuthSelectionSource,
        alternatives: Vec<AuthSelectionSource>,
        selection_basis: AuthSelectionBasis,
    },
    UnsupportedOnly {
        first_reason: String,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthSelectionBasis {
    /// Selected from a supported component security scheme after ambiguity checks.
    ComponentSecurityScheme,
    /// Inferred from repeated required auth-like query parameters when no component security scheme is usable.
    QueryParameterHeuristic,
    /// User explicitly selected a component security scheme by name.
    ExplicitScheme,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AuthSelectionSource {
    SecurityScheme { name: String },
    QueryParameterHeuristic { param_name: String },
}

#[allow(dead_code)]
pub(super) fn derive_auth_kind(spec: &PpSpec) -> Result<AuthKind> {
    Ok(derive_auth_plan(spec)?.selected)
}

pub(super) fn derive_auth_plan(spec: &PpSpec) -> Result<AuthPlan> {
    derive_auth_plan_with_policy(spec, &AuthSelectionPolicy::default())
}

pub(crate) fn derive_auth_plan_with_policy(
    spec: &PpSpec,
    policy: &AuthSelectionPolicy,
) -> Result<AuthPlan> {
    let requirements = derive_requirement_model(spec);
    let Some(security_schemes) = spec
        .document()
        .pointer("/components/securitySchemes")
        .and_then(Value::as_object)
    else {
        return match policy {
            AuthSelectionPolicy::ExplicitScheme { name } => {
                bail!("auth scheme '{name}' was not found in components.securitySchemes")
            }
            AuthSelectionPolicy::FailAmbiguous => Ok(plan_from_query_heuristic(spec, requirements)),
        };
    };
    if security_schemes.is_empty() {
        return match policy {
            AuthSelectionPolicy::ExplicitScheme { name } => {
                bail!("auth scheme '{name}' was not found in components.securitySchemes")
            }
            AuthSelectionPolicy::FailAmbiguous => Ok(plan_from_query_heuristic(spec, requirements)),
        };
    }

    let mut candidates = Vec::new();
    let mut unsupported = Vec::new();

    for (name, scheme) in security_schemes {
        if let Some(reference) = scheme.get("$ref").and_then(Value::as_str) {
            let reason = format!("$ref security scheme not supported in MVP: {reference}");
            let index = candidates.len();
            candidates.push(AuthCandidate::SecurityScheme {
                name: name.clone(),
                auth_kind: AuthKind::Unsupported {
                    reason: reason.clone(),
                },
                supported: false,
                reason: Some(reason),
            });
            unsupported.push(index);
            continue;
        }
        let auth_kind = auth_kind_for_scheme(scheme);
        let supported = is_supported_auth_kind(&auth_kind);
        let reason = unsupported_reason(&auth_kind);
        let index = candidates.len();
        candidates.push(AuthCandidate::SecurityScheme {
            name: name.clone(),
            auth_kind,
            supported,
            reason,
        });
        if !supported {
            unsupported.push(index);
        }
    }

    let selectable_component_indexes = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| candidate.selectable_auth_kind().map(|_| index))
        .collect::<Vec<_>>();

    match policy {
        AuthSelectionPolicy::FailAmbiguous => {
            if selectable_component_indexes.len() > 1 {
                let names = selectable_component_indexes
                    .iter()
                    .filter_map(|index| candidates[*index].component_scheme_name())
                    .collect::<Vec<_>>();
                bail!(
                    "ambiguous auth schemes: {}; pass --auth-scheme <name> to select one explicitly",
                    names.join(", ")
                );
            }
            if let Some((selected, decision)) = select_from_candidate_indexes(
                &candidates,
                selectable_component_indexes,
                AuthSelectionBasis::ComponentSecurityScheme,
            ) {
                return Ok(AuthPlan {
                    candidates,
                    requirements,
                    decision,
                    selected,
                });
            }
        }
        AuthSelectionPolicy::ExplicitScheme { name } => {
            let Some(index) = candidates
                .iter()
                .position(|candidate| candidate.component_scheme_name() == Some(name.as_str()))
            else {
                bail!("auth scheme '{name}' was not found in components.securitySchemes");
            };
            let Some(selected) = candidates[index].selectable_auth_kind() else {
                let reason = candidates[index].unsupported_reason().unwrap_or_else(|| {
                    "security scheme is not supported for generated auth".to_string()
                });
                bail!("auth scheme '{name}' is not supported: {reason}");
            };
            let source = candidates[index]
                .selection_source()
                .expect("selectable component auth has a selection source");
            return Ok(AuthPlan {
                candidates,
                requirements,
                decision: AuthDecision::Selected {
                    source,
                    selection_basis: AuthSelectionBasis::ExplicitScheme,
                },
                selected,
            });
        }
    }

    if let Some(first_unsupported) = unsupported.first() {
        let selected = candidates[*first_unsupported]
            .selectable_or_unsupported_auth_kind()
            .unwrap_or(AuthKind::None);
        let first_reason = unsupported_reason(&selected).unwrap_or_default();
        return Ok(AuthPlan {
            candidates,
            requirements,
            decision: AuthDecision::UnsupportedOnly { first_reason },
            selected,
        });
    }

    let query_plan = plan_from_query_heuristic(spec, requirements.clone());
    candidates.extend(query_plan.candidates);
    Ok(AuthPlan {
        candidates,
        requirements,
        decision: query_plan.decision,
        selected: query_plan.selected,
    })
}

fn plan_from_query_heuristic(spec: &PpSpec, requirements: AuthRequirementModel) -> AuthPlan {
    let candidates = derive_query_api_key_candidates(spec);
    if let Some((selected, decision)) = select_from_candidate_indexes(
        &candidates,
        candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| candidate.selectable_auth_kind().map(|_| index))
            .collect(),
        AuthSelectionBasis::QueryParameterHeuristic,
    ) {
        return AuthPlan {
            candidates,
            requirements,
            decision,
            selected,
        };
    }

    AuthPlan {
        candidates,
        requirements,
        decision: AuthDecision::None,
        selected: AuthKind::None,
    }
}

fn derive_requirement_model(spec: &PpSpec) -> AuthRequirementModel {
    let global = spec
        .root_security_requirements()
        .map(requirement_alternatives)
        .unwrap_or_default();
    let mut operations_inheriting_global = 0;
    let mut operation_overrides = Vec::new();

    for operation_ref in traversal::operations(spec) {
        match operation_ref.security_requirement_names() {
            Some(requirements) => operation_overrides.push(AuthOperationRequirement {
                method: operation_ref.method.to_string(),
                path: operation_ref.path.to_string(),
                operation_id: operation_ref.raw_operation_id(),
                requirements: requirement_alternatives(requirements),
            }),
            None if !global.is_empty() => operations_inheriting_global += 1,
            None => {}
        }
    }

    AuthRequirementModel {
        global,
        operations_inheriting_global,
        operation_overrides,
    }
}

fn requirement_alternatives(requirements: Vec<Vec<String>>) -> Vec<AuthRequirementAlternative> {
    requirements
        .into_iter()
        .map(|scheme_names| AuthRequirementAlternative { scheme_names })
        .collect()
}

fn select_from_candidate_indexes(
    candidates: &[AuthCandidate],
    selectable_indexes: Vec<usize>,
    selection_basis: AuthSelectionBasis,
) -> Option<(AuthKind, AuthDecision)> {
    let first = *selectable_indexes.first()?;
    let selected = candidates[first].selectable_auth_kind()?;
    let selected_source = candidates[first].selection_source()?;
    let alternatives = selectable_indexes
        .iter()
        .skip(1)
        .filter_map(|index| candidates[*index].selection_source())
        .collect::<Vec<_>>();
    let decision = if alternatives.is_empty() {
        AuthDecision::Selected {
            source: selected_source,
            selection_basis,
        }
    } else {
        AuthDecision::AmbiguousSelected {
            selected_source,
            alternatives,
            selection_basis,
        }
    };
    Some((selected, decision))
}

impl AuthCandidate {
    fn selectable_auth_kind(&self) -> Option<AuthKind> {
        match self {
            AuthCandidate::SecurityScheme {
                auth_kind,
                supported: true,
                ..
            } => Some(auth_kind.clone()),
            AuthCandidate::QueryParameterHeuristic {
                param_name,
                accepted: true,
                ..
            } => Some(AuthKind::QueryApiKey {
                param_name: param_name.clone(),
            }),
            _ => None,
        }
    }

    fn selectable_or_unsupported_auth_kind(&self) -> Option<AuthKind> {
        self.selectable_auth_kind().or_else(|| match self {
            AuthCandidate::SecurityScheme { auth_kind, .. } => Some(auth_kind.clone()),
            _ => None,
        })
    }

    fn component_scheme_name(&self) -> Option<&str> {
        match self {
            AuthCandidate::SecurityScheme { name, .. } => Some(name.as_str()),
            AuthCandidate::QueryParameterHeuristic { .. } => None,
        }
    }

    fn unsupported_reason(&self) -> Option<String> {
        match self {
            AuthCandidate::SecurityScheme {
                auth_kind, reason, ..
            } => reason.clone().or_else(|| unsupported_reason(auth_kind)),
            _ => None,
        }
    }

    fn selection_source(&self) -> Option<AuthSelectionSource> {
        match self {
            AuthCandidate::SecurityScheme {
                name,
                supported: true,
                ..
            } => Some(AuthSelectionSource::SecurityScheme { name: name.clone() }),
            AuthCandidate::QueryParameterHeuristic {
                param_name,
                accepted: true,
                ..
            } => Some(AuthSelectionSource::QueryParameterHeuristic {
                param_name: param_name.clone(),
            }),
            _ => None,
        }
    }
}

fn derive_query_api_key_candidates(spec: &PpSpec) -> Vec<AuthCandidate> {
    let operations = traversal::operations(spec);

    if operations.is_empty() {
        return Vec::new();
    }

    let mut candidates: indexmap::IndexMap<String, QueryAuthStats> = indexmap::IndexMap::new();
    let mut first_required_query_names = Vec::new();

    for operation_ref in operations {
        let parameters = operation_ref.parameters();
        let required_query_params = required_query_params(parameters.iter());
        let Some(first_param) = required_query_params.first() else {
            first_required_query_names.push(None);
            continue;
        };
        first_required_query_names.push(first_param.name().map(str::to_string));

        for param in required_query_params {
            let Some(name) = param.name() else {
                continue;
            };
            if !is_auth_query_param_name(name) {
                continue;
            }
            let key = name.to_ascii_lowercase();
            let stats = candidates.entry(key).or_insert_with(|| QueryAuthStats {
                param_name: name.to_string(),
                appearances: 0,
            });
            stats.appearances += 1;
        }
    }

    let operation_count = first_required_query_names.len();
    candidates
        .values()
        .map(|stats| {
            let appears_in_half = stats.appearances * 2 >= operation_count;
            let first_in_every_operation = first_required_query_names
                .iter()
                .all(|name| name.as_deref() == Some(stats.param_name.as_str()));
            AuthCandidate::QueryParameterHeuristic {
                param_name: stats.param_name.clone(),
                appearances: stats.appearances,
                operation_count,
                accepted: appears_in_half || first_in_every_operation,
            }
        })
        .collect()
}

#[derive(Debug)]
struct QueryAuthStats {
    param_name: String,
    appearances: usize,
}

fn required_query_params<'a>(
    params: impl Iterator<Item = &'a crate::spec::PpParameterRef<'a>>,
) -> Vec<crate::spec::PpParameter<'a>> {
    params
        .filter_map(|param| {
            param.item().and_then(|param| {
                (param.location() == Some(PpParameterLocation::Query) && param.required())
                    .then_some(param)
            })
        })
        .collect()
}

fn is_auth_query_param_name(name: &str) -> bool {
    let folded_name: String = name
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        folded_name.as_str(),
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

fn auth_kind_for_scheme(scheme: &Value) -> AuthKind {
    match scheme.get("type").and_then(Value::as_str).unwrap_or("") {
        "http" => match scheme.get("scheme").and_then(Value::as_str).unwrap_or("") {
            value if value.eq_ignore_ascii_case("bearer") => AuthKind::Bearer,
            value if value.eq_ignore_ascii_case("basic") => AuthKind::HttpBasic,
            value => AuthKind::Unsupported {
                reason: format!(
                    "http auth scheme '{value}' not supported in MVP (only bearer/basic)"
                ),
            },
        },
        "apiKey" => match scheme.get("in").and_then(Value::as_str).unwrap_or("") {
            "header" => AuthKind::ApiKey {
                header_name: scheme
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            },
            other => AuthKind::Unsupported {
                reason: format!("apiKey in '{other}' not supported in MVP (only header)"),
            },
        },
        "oauth2" => AuthKind::Unsupported {
            reason: "OAuth2 flows are not implemented; use an explicit bearer security scheme"
                .into(),
        },
        "openIdConnect" => AuthKind::Unsupported {
            reason: "OpenID Connect not supported in MVP".into(),
        },
        other => AuthKind::Unsupported {
            reason: format!("security scheme type '{other}' not supported in MVP"),
        },
    }
}

fn is_supported_auth_kind(auth_kind: &AuthKind) -> bool {
    matches!(
        auth_kind,
        AuthKind::Bearer
            | AuthKind::HttpBasic
            | AuthKind::ApiKey { .. }
            | AuthKind::QueryApiKey { .. }
    )
}

fn unsupported_reason(auth_kind: &AuthKind) -> Option<String> {
    match auth_kind {
        AuthKind::Unsupported { reason } => Some(reason.clone()),
        _ => None,
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
        let spec = crate::spec::parse_spec_for_tests(BEARER_SPEC).unwrap();
        assert_eq!(derive_auth_kind(&spec).unwrap(), AuthKind::Bearer);
    }

    #[test]
    fn auth_plan_documents_unambiguous_supported_auth() {
        let spec = crate::spec::parse_spec_for_tests(BEARER_SPEC).unwrap();
        let plan = derive_auth_plan(&spec).unwrap();

        assert_eq!(plan.selected, AuthKind::Bearer);
        assert_eq!(
            plan.decision,
            AuthDecision::Selected {
                source: AuthSelectionSource::SecurityScheme {
                    name: "bearerAuth".into()
                },
                selection_basis: AuthSelectionBasis::ComponentSecurityScheme,
            }
        );
        assert_eq!(
            plan.candidates,
            vec![AuthCandidate::SecurityScheme {
                name: "bearerAuth".into(),
                auth_kind: AuthKind::Bearer,
                supported: true,
                reason: None,
            }]
        );
    }

    #[test]
    fn http_basic_auth_detected() {
        let spec = crate::spec::parse_spec_for_tests(
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
    fn ambiguous_multi_scheme_plan_fails_by_default() {
        let spec = crate::spec::parse_spec_for_tests(
            r#"
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
    bearerAuth:
      type: http
      scheme: bearer
"#,
        )
        .unwrap();

        let err = derive_auth_plan(&spec).unwrap_err().to_string();
        assert!(err.contains("ambiguous auth schemes: apiKeyAuth, bearerAuth"));
        assert!(err.contains("--auth-scheme <name>"));
    }

    #[test]
    fn default_auth_policy_is_fail_ambiguous() {
        assert!(matches!(
            AuthSelectionPolicy::default(),
            AuthSelectionPolicy::FailAmbiguous
        ));
    }

    #[test]
    fn fail_ambiguous_policy_errors_on_multiple_selectable_component_schemes() {
        let spec = crate::spec::parse_spec_for_tests(
            r#"
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
    bearerAuth:
      type: http
      scheme: bearer
"#,
        )
        .unwrap();

        let err = derive_auth_plan_with_policy(&spec, &AuthSelectionPolicy::FailAmbiguous)
            .unwrap_err()
            .to_string();
        assert!(err.contains("ambiguous auth schemes: apiKeyAuth, bearerAuth"));
        assert!(err.contains("--auth-scheme <name>"));
    }

    #[test]
    fn explicit_policy_selects_named_component_scheme() {
        let spec = crate::spec::parse_spec_for_tests(
            r#"
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
    bearerAuth:
      type: http
      scheme: bearer
"#,
        )
        .unwrap();

        let plan = derive_auth_plan_with_policy(
            &spec,
            &AuthSelectionPolicy::ExplicitScheme {
                name: "bearerAuth".into(),
            },
        )
        .unwrap();

        assert_eq!(plan.selected, AuthKind::Bearer);
        assert_eq!(
            plan.decision,
            AuthDecision::Selected {
                source: AuthSelectionSource::SecurityScheme {
                    name: "bearerAuth".into()
                },
                selection_basis: AuthSelectionBasis::ExplicitScheme,
            }
        );
    }

    #[test]
    fn explicit_policy_errors_when_scheme_is_missing() {
        let spec = crate::spec::parse_spec_for_tests(BEARER_SPEC).unwrap();

        let err = derive_auth_plan_with_policy(
            &spec,
            &AuthSelectionPolicy::ExplicitScheme {
                name: "missingAuth".into(),
            },
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("auth scheme 'missingAuth' was not found"));
    }

    #[test]
    fn explicit_policy_errors_when_scheme_is_unsupported() {
        let spec = crate::spec::parse_spec_for_tests(
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
"#,
        )
        .unwrap();

        let err = derive_auth_plan_with_policy(
            &spec,
            &AuthSelectionPolicy::ExplicitScheme {
                name: "digestAuth".into(),
            },
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("auth scheme 'digestAuth' is not supported"));
        assert!(err.contains("http auth scheme 'digest' not supported"));
    }

    #[test]
    fn auth_plan_records_security_requirements_for_explicit_selection() {
        let spec = crate::spec::parse_spec_for_tests(
            r#"
openapi: 3.0.0
info:
  title: My API
  version: "1.0.0"
security:
  - bearerAuth: []
paths:
  /ping:
    get:
      operationId: getPing
      responses:
        '200':
          description: ok
components:
  securitySchemes:
    apiKeyAuth:
      type: apiKey
      in: header
      name: X-API-Key
    bearerAuth:
      type: http
      scheme: bearer
"#,
        )
        .unwrap();

        let plan = derive_auth_plan_with_policy(
            &spec,
            &AuthSelectionPolicy::ExplicitScheme {
                name: "bearerAuth".into(),
            },
        )
        .unwrap();

        assert_eq!(plan.selected, AuthKind::Bearer);
        assert_eq!(
            plan.requirements.global,
            vec![AuthRequirementAlternative {
                scheme_names: vec!["bearerAuth".into()]
            }]
        );
        assert_eq!(plan.requirements.operations_inheriting_global, 1);
        assert!(plan.requirements.operation_overrides.is_empty());
    }

    #[test]
    fn oauth2_first_bearer_second_selects_supported_scheme() {
        let spec = crate::spec::parse_spec_for_tests(
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
    fn oauth2_first_apikey_second_selects_supported_scheme() {
        let spec = crate::spec::parse_spec_for_tests(
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
    apiKeyAuth:
      type: apiKey
      in: header
      name: X-API-Key
"#,
        )
        .unwrap();

        let plan = derive_auth_plan(&spec).unwrap();
        assert_eq!(
            plan.selected,
            AuthKind::ApiKey {
                header_name: "X-API-Key".into()
            }
        );

        let explicit_plan = derive_auth_plan_with_policy(
            &spec,
            &AuthSelectionPolicy::ExplicitScheme {
                name: "apiKeyAuth".into(),
            },
        )
        .unwrap();
        assert_eq!(
            explicit_plan.selected,
            AuthKind::ApiKey {
                header_name: "X-API-Key".into()
            }
        );
    }

    #[test]
    fn oauth2_only_plan_is_unsupported() {
        let spec = crate::spec::parse_spec_for_tests(
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

        let reason = "OAuth2 flows are not implemented; use an explicit bearer security scheme";
        let plan = derive_auth_plan(&spec).unwrap();
        assert_eq!(
            plan.selected,
            AuthKind::Unsupported {
                reason: reason.into()
            }
        );
        assert_eq!(derive_auth_kind(&spec).unwrap(), plan.selected);
        assert_eq!(
            plan.decision,
            AuthDecision::UnsupportedOnly {
                first_reason: reason.into()
            }
        );
        assert_eq!(
            plan.candidates,
            vec![AuthCandidate::SecurityScheme {
                name: "oauth2".into(),
                auth_kind: AuthKind::Unsupported {
                    reason: reason.into()
                },
                supported: false,
                reason: Some(reason.into()),
            }]
        );

        let err = derive_auth_plan_with_policy(
            &spec,
            &AuthSelectionPolicy::ExplicitScheme {
                name: "oauth2".into(),
            },
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("auth scheme 'oauth2' is not supported"));
        assert!(err.contains(reason));
    }

    #[test]
    fn all_unsupported_auth_returns_first_unsupported() {
        let spec = crate::spec::parse_spec_for_tests(
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
    fn unsupported_only_plan_documents_first_reason() {
        let spec = crate::spec::parse_spec_for_tests(
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

        let plan = derive_auth_plan(&spec).unwrap();
        assert_eq!(
            plan.selected,
            AuthKind::Unsupported {
                reason: "http auth scheme 'digest' not supported in MVP (only bearer/basic)".into()
            }
        );
        assert_eq!(
            plan.decision,
            AuthDecision::UnsupportedOnly {
                first_reason: "http auth scheme 'digest' not supported in MVP (only bearer/basic)"
                    .into()
            }
        );
        assert_eq!(plan.candidates.len(), 2);
    }

    #[test]
    fn apikey_header_detected() {
        let spec = crate::spec::parse_spec_for_tests(APIKEY_SPEC).unwrap();
        assert_eq!(
            derive_auth_kind(&spec).unwrap(),
            AuthKind::ApiKey {
                header_name: "X-API-Key".into()
            }
        );
    }

    #[test]
    fn required_license_query_param_in_all_ops_detects_query_api_key() {
        let spec = crate::spec::parse_spec_for_tests(
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
    fn query_api_key_plan_documents_heuristic_basis() {
        let spec = crate::spec::parse_spec_for_tests(
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
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();

        let plan = derive_auth_plan(&spec).unwrap();
        assert_eq!(
            plan.selected,
            AuthKind::QueryApiKey {
                param_name: "license".into()
            }
        );
        assert_eq!(
            plan.decision,
            AuthDecision::Selected {
                source: AuthSelectionSource::QueryParameterHeuristic {
                    param_name: "license".into()
                },
                selection_basis: AuthSelectionBasis::QueryParameterHeuristic,
            }
        );
        assert_eq!(
            plan.candidates,
            vec![AuthCandidate::QueryParameterHeuristic {
                param_name: "license".into(),
                appearances: 1,
                operation_count: 1,
                accepted: true,
            }]
        );
    }

    #[test]
    fn path_level_query_api_key_detects_query_api_key() {
        let spec = crate::spec::parse_spec_for_tests(
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
        let spec = crate::spec::parse_spec_for_tests(
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
        let plan = derive_auth_plan(&spec).unwrap();
        assert_eq!(plan.selected, AuthKind::None);
        assert_eq!(plan.decision, AuthDecision::None);
        assert_eq!(
            plan.candidates,
            vec![AuthCandidate::QueryParameterHeuristic {
                param_name: "token".into(),
                appearances: 1,
                operation_count: 3,
                accepted: false,
            }]
        );
    }
}
