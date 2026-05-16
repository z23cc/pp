use anyhow::Result;
use openapiv3::OpenAPI;

use crate::backend::BackendCapabilities;

use super::report::ReportEntry;
use super::transform::TransformAuditEntry;

mod operation_naming;
mod progenitor_compatibility;
mod response_relaxation;

#[cfg(test)]
use openapiv3::{QueryStyle, ReferenceOr, SchemaKind, StatusCode, Type};
#[cfg(test)]
use progenitor_compatibility::{FORM_MIME, JSON_MIME};

#[derive(Debug, Clone)]
pub(crate) struct TypedNormalizationPlan {
    operation_naming: operation_naming::OperationNamingPlan,
    compatibility: progenitor_compatibility::CompatibilityTransformPlan,
    response_relaxation: response_relaxation::ResponseRelaxationPlan,
}

impl TypedNormalizationPlan {
    pub(crate) fn report_entries(&self) -> Vec<ReportEntry> {
        let mut reports = Vec::new();
        reports.extend(self.operation_naming.report_entries());
        reports.extend(self.compatibility.report_entries());
        reports.extend(self.response_relaxation.report_entries());
        reports
    }

    pub(crate) fn audit_entries(&self) -> Vec<TransformAuditEntry> {
        let mut audits = Vec::new();
        audits.extend(self.operation_naming.audit_entries());
        audits.extend(self.compatibility.audit_entries());
        audits.extend(self.response_relaxation.audit_entries());
        audits
    }
}

pub(crate) fn propose_typed_normalization_transforms(
    spec: &OpenAPI,
    backend_capabilities: &BackendCapabilities,
) -> TypedNormalizationPlan {
    let operation_naming = operation_naming::propose(spec);

    let mut compatibility_basis = spec.clone();
    let mut discarded_reports = Vec::new();
    operation_naming::apply_approved(
        &mut compatibility_basis,
        &mut discarded_reports,
        &operation_naming,
    );
    let compatibility =
        progenitor_compatibility::propose_transforms(&compatibility_basis, backend_capabilities);

    let mut response_basis = compatibility_basis;
    discarded_reports.clear();
    let _ = progenitor_compatibility::apply_approved(
        &mut response_basis,
        &mut discarded_reports,
        backend_capabilities,
        &compatibility,
    )
    .expect("compatibility proposal replay for response relaxation should not fail");
    let response_relaxation = response_relaxation::propose(&response_basis, backend_capabilities);

    TypedNormalizationPlan {
        operation_naming,
        compatibility,
        response_relaxation,
    }
}

pub(crate) fn normalize_with_approved_typed_normalization_transforms(
    spec: &mut OpenAPI,
    backend_capabilities: &BackendCapabilities,
    approved_transforms: &TypedNormalizationPlan,
) -> Result<Vec<ReportEntry>> {
    let mut reports = Vec::new();
    operation_naming::apply_approved(spec, &mut reports, &approved_transforms.operation_naming);
    let compatibility_stats = progenitor_compatibility::apply_approved(
        spec,
        &mut reports,
        backend_capabilities,
        &approved_transforms.compatibility,
    )?;
    progenitor_compatibility::emit_summary_reports(&mut reports, &compatibility_stats);
    progenitor_compatibility::emit_optional_object_query_param_report(
        &mut reports,
        &compatibility_stats,
    );
    response_relaxation::apply_approved(
        spec,
        &mut reports,
        &approved_transforms.response_relaxation,
    );

    Ok(reports)
}

#[cfg(test)]
pub(crate) fn propose_compatibility_transforms(
    spec: &OpenAPI,
    backend_capabilities: &BackendCapabilities,
) -> progenitor_compatibility::CompatibilityTransformPlan {
    progenitor_compatibility::propose_transforms(spec, backend_capabilities)
}

#[cfg(test)]
pub(crate) fn normalize_with_approved_compatibility_transforms(
    spec: &mut OpenAPI,
    backend_capabilities: &BackendCapabilities,
    approved_compatibility_transforms: &progenitor_compatibility::CompatibilityTransformPlan,
) -> Result<Vec<ReportEntry>> {
    let approved_transforms = TypedNormalizationPlan {
        operation_naming: operation_naming::propose(spec),
        compatibility: approved_compatibility_transforms.clone(),
        response_relaxation: response_relaxation::ResponseRelaxationPlan::default(),
    };
    normalize_with_approved_typed_normalization_transforms(
        spec,
        backend_capabilities,
        &approved_transforms,
    )
}

#[cfg(test)]
fn normalize_unchecked_for_tests(
    spec: &mut OpenAPI,
    backend_capabilities: &BackendCapabilities,
) -> Result<Vec<ReportEntry>> {
    let approved_transforms = propose_typed_normalization_transforms(spec, backend_capabilities);
    normalize_with_approved_typed_normalization_transforms(
        spec,
        backend_capabilities,
        &approved_transforms,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendCapabilities;
    use crate::spec::normalization_rules::typed;
    use crate::spec::report::{ReportStage, ReportSubject};
    use crate::spec::transform::{
        TransformActionKind, TransformAuditEntry, TransformPlan, TransformPolicy,
    };

    fn assert_strict_rejects_typed_proposal_without_mutating_spec(
        yaml: &str,
        expected_codes: &[&str],
    ) {
        let spec: OpenAPI = serde_yaml::from_str(yaml).unwrap();
        let before = serde_json::to_value(&spec).unwrap();
        let proposals =
            propose_typed_normalization_transforms(&spec, &BackendCapabilities::progenitor());
        let proposal_reports = proposals.report_entries();
        let mut plan = TransformPlan::from_reports(&proposal_reports);

        assert!(
            plan.approve(&TransformPolicy::strict()).is_err(),
            "strict policy unexpectedly approved reports: {proposal_reports:?}"
        );
        assert_eq!(serde_json::to_value(&spec).unwrap(), before);
        for expected_code in expected_codes {
            assert!(
                proposal_reports
                    .iter()
                    .any(|report| report.code == *expected_code),
                "missing proposed report code {expected_code}; reports: {proposal_reports:?}"
            );
        }
    }

    #[test]
    fn typed_plan_includes_operation_id_shortening_before_apply() {
        let spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Verbose Operation
  version: "1.0.0"
paths:
  /capabilities:
    get:
      operationId: PlausibleWeb.Plugins.API.Controllers.Capabilities.index
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();
        let before = serde_json::to_value(&spec).unwrap();

        let proposals =
            propose_typed_normalization_transforms(&spec, &BackendCapabilities::progenitor());
        let proposal_reports = proposals.report_entries();
        let mut plan = TransformPlan::from_reports(&proposal_reports);
        plan.approve(&TransformPolicy::strict())
            .expect("operation ID shortening is lossless and strict-approved");

        assert_eq!(serde_json::to_value(&spec).unwrap(), before);
        assert_eq!(proposal_reports.len(), 1);
        assert_eq!(proposal_reports[0].code, typed::OPERATION_IDS_SHORTENED);
        assert_eq!(proposals.audit_entries().len(), 1);
        assert_eq!(
            proposals.audit_entries()[0],
            TransformAuditEntry::new(
                "typed_normalization",
                typed::OPERATION_IDS_SHORTENED,
                "operation get /capabilities operationId",
                "shorten operationId",
            )
            .with_target_pointer("/paths/~1capabilities/get/operationId")
            .with_action_kind(TransformActionKind::Rename)
            .with_before_after(
                "PlausibleWeb.Plugins.API.Controllers.Capabilities.index",
                "capabilities_index",
            )
            .with_before_after_json(
                serde_json::json!("PlausibleWeb.Plugins.API.Controllers.Capabilities.index"),
                serde_json::json!("capabilities_index"),
            )
        );
        assert_eq!(
            proposal_reports[0].subject,
            Some(ReportSubject::operation(
                "PlausibleWeb.Plugins.API.Controllers.Capabilities.index"
            ))
        );
        assert_eq!(
            plan.approval.unwrap().decisions[0].allowed_by,
            "strict_default"
        );
    }

    #[test]
    fn downstream_typed_reports_use_post_operation_naming_basis() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Verbose Pruning
  version: "1.0.0"
paths:
  /capabilities:
    get:
      operationId: PlausibleWeb.Plugins.API.Controllers.Capabilities.index
      responses:
        '200':
          description: ok
        '404':
          description: missing
"#,
        )
        .unwrap();

        let approved_plan =
            propose_typed_normalization_transforms(&spec, &BackendCapabilities::progenitor());
        let proposal_reports = approved_plan.report_entries();
        assert!(proposal_reports
            .iter()
            .any(|report| report.contains("capabilities_index responses")));
        assert!(!proposal_reports.iter().any(|report| report
            .contains("PlausibleWeb.Plugins.API.Controllers.Capabilities.index responses")));

        let reports = normalize_with_approved_typed_normalization_transforms(
            &mut spec,
            &BackendCapabilities::progenitor(),
            &approved_plan,
        )
        .unwrap();

        assert_eq!(reports, proposal_reports);
    }

    #[test]
    fn operation_id_shortening_applies_only_from_approved_typed_plan() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Verbose Operation
  version: "1.0.0"
paths:
  /capabilities:
    get:
      operationId: PlausibleWeb.Plugins.API.Controllers.Capabilities.index
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();
        let empty_plan = TypedNormalizationPlan {
            operation_naming: operation_naming::OperationNamingPlan::default(),
            compatibility: progenitor_compatibility::CompatibilityTransformPlan::default(),
            response_relaxation: response_relaxation::ResponseRelaxationPlan::default(),
        };

        let reports = normalize_with_approved_typed_normalization_transforms(
            &mut spec,
            &BackendCapabilities::progenitor(),
            &empty_plan,
        )
        .unwrap();
        let path = spec.paths.paths.get("/capabilities").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };
        assert_eq!(
            path.get.as_ref().unwrap().operation_id.as_deref(),
            Some("PlausibleWeb.Plugins.API.Controllers.Capabilities.index")
        );
        assert!(!reports
            .iter()
            .any(|report| report.code == typed::OPERATION_IDS_SHORTENED));

        let approved_plan =
            propose_typed_normalization_transforms(&spec, &BackendCapabilities::progenitor());
        let reports = normalize_with_approved_typed_normalization_transforms(
            &mut spec,
            &BackendCapabilities::progenitor(),
            &approved_plan,
        )
        .unwrap();
        let path = spec.paths.paths.get("/capabilities").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };
        assert_eq!(
            path.get.as_ref().unwrap().operation_id.as_deref(),
            Some("capabilities_index")
        );
        assert!(reports
            .iter()
            .any(|report| report.code == typed::OPERATION_IDS_SHORTENED));
    }

    #[test]
    fn planned_pruning_reports_are_policy_checked_without_mutating_spec() {
        let spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Planned Pruning
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: listItems
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
            application/xml:
              schema:
                type: object
        '404':
          description: missing
"#,
        )
        .unwrap();
        let before = serde_json::to_value(&spec).unwrap();

        let proposals = propose_compatibility_transforms(&spec, &BackendCapabilities::progenitor());
        let proposal_reports = proposals.report_entries();
        let mut plan = TransformPlan::from_reports(&proposal_reports);

        assert!(plan.approve(&TransformPolicy::strict()).is_err());
        assert_eq!(serde_json::to_value(&spec).unwrap(), before);
        assert_eq!(proposal_reports.len(), 2);
        assert!(proposal_reports
            .iter()
            .any(|report| report.code == typed::RESPONSE_VARIANTS_PRUNED));
        assert!(proposal_reports
            .iter()
            .any(|report| report.code == typed::CONTENT_TYPES_PRUNED));
    }

    #[test]
    fn strict_policy_rejects_schema_default_proposal_before_mutation() {
        assert_strict_rejects_typed_proposal_without_mutating_spec(
            r#"
openapi: 3.0.0
info:
  title: Defaults
  version: "1.0.0"
paths: {}
components:
  schemas:
    Pet:
      type: object
      default:
        name: cat
"#,
            &[typed::SCHEMA_DEFAULTS_DROPPED],
        );
    }

    #[test]
    fn strict_policy_rejects_query_param_proposals_before_mutation() {
        assert_strict_rejects_typed_proposal_without_mutating_spec(
            r##"
openapi: 3.0.0
info:
  title: Query Params
  version: "1.0.0"
paths:
  /search:
    get:
      operationId: searchPets
      parameters:
        - name: filter
          in: query
          schema:
            $ref: "#/components/schemas/Filter"
        - name: required_filter
          in: query
          required: true
          style: deepObject
          schema:
            type: object
      responses:
        '200':
          description: ok
components:
  schemas:
    Filter:
      type: object
      properties:
        color:
          type: string
"##,
            &[
                typed::OPTIONAL_OBJECT_QUERY_PARAMS_DROPPED,
                typed::DEEP_OBJECT_QUERY_PARAMS_REWRITTEN,
            ],
        );
    }

    #[test]
    fn strict_policy_preserves_empty_request_body_content_without_proposal() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Empty Request Bodies
  version: "1.0.0"
paths:
  /pets:
    post:
      operationId: createPet
      requestBody:
        content: {}
      responses:
        '200':
          description: ok
components:
  requestBodies:
    EmptyBody:
      content: {}
"#,
        )
        .unwrap();
        let before = serde_json::to_value(&spec).unwrap();
        let approved_plan =
            propose_typed_normalization_transforms(&spec, &BackendCapabilities::progenitor());
        let proposal_reports = approved_plan.report_entries();
        let mut transform_plan = TransformPlan::from_reports(&proposal_reports);

        transform_plan
            .approve(&TransformPolicy::strict())
            .expect("empty request body content has no implicit normalization proposal");
        let reports = normalize_with_approved_typed_normalization_transforms(
            &mut spec,
            &BackendCapabilities::progenitor(),
            &approved_plan,
        )
        .unwrap();

        assert!(proposal_reports.is_empty());
        assert!(reports.is_empty());
        assert_eq!(serde_json::to_value(&spec).unwrap(), before);
    }

    #[test]
    fn strict_policy_rejects_schemaless_body_proposal_before_mutation() {
        assert_strict_rejects_typed_proposal_without_mutating_spec(
            r#"
openapi: 3.0.0
info:
  title: Schemaless Body
  version: "1.0.0"
paths:
  /pets:
    post:
      operationId: createPet
      requestBody:
        content:
          application/json: {}
      responses:
        '200':
          description: ok
"#,
            &[typed::SCHEMALESS_REQUEST_BODY_DROPPED],
        );
    }

    #[test]
    fn strict_policy_rejects_response_relaxation_proposal_before_mutation() {
        assert_strict_rejects_typed_proposal_without_mutating_spec(
            r#"
openapi: 3.0.0
info:
  title: Relax Responses
  version: "1.0.0"
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
                required: [name]
                properties:
                  name:
                    type: string
"#,
            &[typed::RESPONSE_SCHEMAS_RELAXED],
        );
    }

    #[test]
    fn strict_policy_rejects_enum_and_property_collision_proposals_before_mutation() {
        assert_strict_rejects_typed_proposal_without_mutating_spec(
            r#"
openapi: 3.0.0
info:
  title: Collisions
  version: "1.0.0"
paths: {}
components:
  schemas:
    Reaction:
      type: string
      enum:
        - "+1"
        - "-1"
    Pet:
      type: object
      properties:
        foo-bar:
          type: string
        foo_bar:
          type: string
"#,
            &[
                typed::ENUM_CONSTRAINT_DROPPED,
                typed::PROPERTIES_COLLIDING_DROPPED,
            ],
        );
    }

    #[test]
    fn strict_policy_rejects_unsupported_schema_type_proposal_before_mutation() {
        assert_strict_rejects_typed_proposal_without_mutating_spec(
            r#"
openapi: 3.0.0
info:
  title: Unsupported Type
  version: "1.0.0"
paths: {}
components:
  schemas:
    Mystery:
      type: ""
      enum:
        - ok
"#,
            &[typed::UNSUPPORTED_SCHEMA_TYPE_REPLACED],
        );
    }

    #[test]
    fn component_content_pruning_reports_component_subjects() {
        let spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Component Pruning
  version: "1.0.0"
paths: {}
components:
  requestBodies:
    Upload:
      content:
        application/json:
          schema:
            type: object
        application/xml:
          schema:
            type: object
  responses:
    Pet:
      description: ok
      content:
        application/json:
          schema:
            type: object
        application/xml:
          schema:
            type: object
"#,
        )
        .unwrap();

        let proposals = propose_compatibility_transforms(&spec, &BackendCapabilities::progenitor());
        let proposal_reports = proposals.report_entries();

        assert!(proposal_reports.iter().any(|report| {
            report.code == typed::CONTENT_TYPES_PRUNED
                && report.subject == Some(ReportSubject::component("component requestBody Upload"))
        }));
        assert!(proposal_reports.iter().any(|report| {
            report.code == typed::CONTENT_TYPES_PRUNED
                && report.subject == Some(ReportSubject::component("component response Pet"))
        }));
    }

    #[test]
    fn approved_pruning_actions_apply_and_can_be_recorded_in_plan() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Approved Pruning
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: listItems
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
            application/xml:
              schema:
                type: object
        '404':
          description: missing
"#,
        )
        .unwrap();

        let proposals = propose_compatibility_transforms(&spec, &BackendCapabilities::progenitor());
        let proposal_reports = proposals.report_entries();
        let mut proposal_plan = TransformPlan::from_reports(&proposal_reports);
        proposal_plan
            .approve(&TransformPolicy::compatibility())
            .expect("compatibility policy approves proposed pruning");

        let reports = normalize_with_approved_compatibility_transforms(
            &mut spec,
            &BackendCapabilities::progenitor(),
            &proposals,
        )
        .unwrap();
        assert_eq!(reports, proposal_reports);
        let mut transform_plan = TransformPlan::from_reports(&reports);
        transform_plan
            .approve(&TransformPolicy::compatibility())
            .expect("applied reports remain approvable");

        let path = spec.paths.paths.get("/items").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };
        let responses = &path.get.as_ref().unwrap().responses;
        assert!(responses.responses.contains_key(&StatusCode::Code(200)));
        assert_eq!(responses.responses.len(), 1);
        let ReferenceOr::Item(response) = responses.responses.get(&StatusCode::Code(200)).unwrap()
        else {
            panic!("expected inline response");
        };
        assert_eq!(
            response.content.keys().cloned().collect::<Vec<_>>(),
            vec![JSON_MIME]
        );
        assert!(transform_plan
            .approval
            .unwrap()
            .decisions
            .iter()
            .any(|decision| {
                decision.code == typed::RESPONSE_VARIANTS_PRUNED
                    && decision.allowed_by == "compatibility_profile"
            }));
    }

    #[test]
    fn response_variants_prefer_200_and_warn() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Multi Response
  version: "1.0.0"
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '404':
          description: missing
        '200':
          description: ok
        default:
          description: fallback
"#,
        )
        .unwrap();

        let warnings =
            normalize_unchecked_for_tests(&mut spec, &BackendCapabilities::progenitor()).unwrap();
        let path = spec.paths.paths.get("/pets").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };
        let responses = &path.get.as_ref().unwrap().responses;

        assert!(responses.responses.contains_key(&StatusCode::Code(200)));
        assert_eq!(responses.responses.len(), 1);
        assert!(responses.default.is_none());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("responses — kept 200"));
        assert!(warnings[0].contains("dropped 404, default"));
    }

    #[test]
    fn unsupported_any_schema_type_is_dropped_and_warns() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Unsupported Type
  version: "1.0.0"
paths: {}
components:
  schemas:
    Mystery:
      type: ""
      enum:
        - ok
"#,
        )
        .unwrap();

        let warnings =
            normalize_unchecked_for_tests(&mut spec, &BackendCapabilities::progenitor()).unwrap();
        let components = spec.components.unwrap();
        let ReferenceOr::Item(schema) = components.schemas.get("Mystery").unwrap() else {
            panic!("expected inline schema");
        };
        let SchemaKind::Any(any) = &schema.schema_kind else {
            panic!("expected any schema");
        };

        assert!(any.typ.is_none());
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].stage, ReportStage::TypedNormalization);
        assert_eq!(warnings[0].code, typed::UNSUPPORTED_SCHEMA_TYPE_REPLACED);
        assert_eq!(
            warnings[0].subject,
            Some(ReportSubject::schema("component schema Mystery"))
        );
        assert!(warnings[0].contains("component schema Mystery"));
        assert!(warnings[0].contains("replaced unsupported type '' with fallback"));
    }

    #[test]
    fn schema_defaults_are_dropped_recursively_and_warn_once() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Defaults
  version: "1.0.0"
paths:
  /pets:
    post:
      operationId: createPet
      parameters:
        - name: limit
          in: query
          schema:
            type: integer
            default: "bad"
      requestBody:
        content:
          application/json:
            schema:
              type: object
              properties:
                name:
                  type: string
                  default: cat
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: array
                items:
                  type: string
                  default: dog
components:
  schemas:
    Pet:
      type: object
      default:
        name: fish
"#,
        )
        .unwrap();

        let warnings =
            normalize_unchecked_for_tests(&mut spec, &BackendCapabilities::progenitor()).unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0], "normalized 4 schemas — dropped default values");

        let components = spec.components.as_ref().unwrap();
        let ReferenceOr::Item(schema) = components.schemas.get("Pet").unwrap() else {
            panic!("expected inline schema");
        };
        assert!(schema.schema_data.default.is_none());

        let path = spec.paths.paths.get("/pets").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };
        let operation = path.post.as_ref().unwrap();
        let ReferenceOr::Item(request_body) = operation.request_body.as_ref().unwrap() else {
            panic!("expected inline request body");
        };
        let request_schema = request_body
            .content
            .get(JSON_MIME)
            .unwrap()
            .schema
            .as_ref()
            .unwrap();
        let ReferenceOr::Item(request_schema) = request_schema else {
            panic!("expected inline request schema");
        };
        assert!(request_schema.schema_data.default.is_none());
    }

    #[test]
    fn enum_sanitization_collision_drops_enum_and_warns() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Enum Collision
  version: "1.0.0"
paths: {}
components:
  schemas:
    Reaction:
      type: string
      enum:
        - "+1"
        - "-1"
"#,
        )
        .unwrap();

        let warnings =
            normalize_unchecked_for_tests(&mut spec, &BackendCapabilities::progenitor()).unwrap();
        let components = spec.components.unwrap();
        let ReferenceOr::Item(schema) = components.schemas.get("Reaction").unwrap() else {
            panic!("expected inline schema");
        };
        let SchemaKind::Type(Type::String(string)) = &schema.schema_kind else {
            panic!("expected string schema");
        };

        assert!(string.enumeration.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("component schema Reaction"));
        assert!(warnings[0].contains("dropped enum constraint"));
        assert!(warnings[0].contains("+1, -1"));
        assert!(warnings[0].contains("preserving wire format"));
    }

    #[test]
    fn response_relaxation_applies_only_from_approved_typed_plan() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Relax Responses
  version: "1.0.0"
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
                required: [name]
                properties:
                  name:
                    type: string
"#,
        )
        .unwrap();
        let empty_plan = TypedNormalizationPlan {
            operation_naming: operation_naming::OperationNamingPlan::default(),
            compatibility: progenitor_compatibility::CompatibilityTransformPlan::default(),
            response_relaxation: response_relaxation::ResponseRelaxationPlan::default(),
        };

        let reports = normalize_with_approved_typed_normalization_transforms(
            &mut spec,
            &BackendCapabilities::progenitor(),
            &empty_plan,
        )
        .unwrap();
        let path = spec.paths.paths.get("/pets").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };
        let response = path
            .get
            .as_ref()
            .unwrap()
            .responses
            .responses
            .get(&StatusCode::Code(200))
            .unwrap();
        let ReferenceOr::Item(response) = response else {
            panic!("expected inline response");
        };
        let ReferenceOr::Item(schema) = response
            .content
            .get(JSON_MIME)
            .unwrap()
            .schema
            .as_ref()
            .unwrap()
        else {
            panic!("expected inline response schema");
        };
        let SchemaKind::Type(Type::Object(object)) = &schema.schema_kind else {
            panic!("expected response object");
        };
        assert_eq!(object.required, vec!["name"]);
        assert!(!reports
            .iter()
            .any(|report| report.code == typed::RESPONSE_SCHEMAS_RELAXED));

        let approved_plan =
            propose_typed_normalization_transforms(&spec, &BackendCapabilities::progenitor());
        let reports = normalize_with_approved_typed_normalization_transforms(
            &mut spec,
            &BackendCapabilities::progenitor(),
            &approved_plan,
        )
        .unwrap();
        let path = spec.paths.paths.get("/pets").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };
        let response = path
            .get
            .as_ref()
            .unwrap()
            .responses
            .responses
            .get(&StatusCode::Code(200))
            .unwrap();
        let ReferenceOr::Item(response) = response else {
            panic!("expected inline response");
        };
        let ReferenceOr::Item(schema) = response
            .content
            .get(JSON_MIME)
            .unwrap()
            .schema
            .as_ref()
            .unwrap()
        else {
            panic!("expected inline response schema");
        };
        let SchemaKind::Type(Type::Object(object)) = &schema.schema_kind else {
            panic!("expected response object");
        };
        assert!(object.required.is_empty());
        assert!(reports
            .iter()
            .any(|report| report.code == typed::RESPONSE_SCHEMAS_RELAXED));
    }

    #[test]
    fn response_schemas_are_relaxed_without_touching_request_body() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Relax Responses
  version: "1.0.0"
paths:
  /pets:
    post:
      operationId: createPet
      requestBody:
        content:
          application/json:
            schema:
              type: object
              required: [name]
              properties:
                name:
                  type: string
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
                required: [name]
                properties:
                  name:
                    type: string
"#,
        )
        .unwrap();

        let warnings =
            normalize_unchecked_for_tests(&mut spec, &BackendCapabilities::progenitor()).unwrap();
        let path = spec.paths.paths.get("/pets").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };
        let operation = path.post.as_ref().unwrap();
        let ReferenceOr::Item(request_body) = operation.request_body.as_ref().unwrap() else {
            panic!("expected inline request body");
        };
        let ReferenceOr::Item(request_schema) = request_body
            .content
            .get(JSON_MIME)
            .unwrap()
            .schema
            .as_ref()
            .unwrap()
        else {
            panic!("expected inline request schema");
        };
        let SchemaKind::Type(Type::Object(request_object)) = &request_schema.schema_kind else {
            panic!("expected request object");
        };
        assert_eq!(request_object.required, vec!["name"]);

        let response = operation
            .responses
            .responses
            .get(&StatusCode::Code(200))
            .unwrap();
        let ReferenceOr::Item(response) = response else {
            panic!("expected inline response");
        };
        let ReferenceOr::Item(response_schema) = response
            .content
            .get(JSON_MIME)
            .unwrap()
            .schema
            .as_ref()
            .unwrap()
        else {
            panic!("expected inline response schema");
        };
        let SchemaKind::Type(Type::Object(response_object)) = &response_schema.schema_kind else {
            panic!("expected response object");
        };
        assert!(response_object.required.is_empty());
        let ReferenceOr::Item(name_schema) = response_object.properties.get("name").unwrap() else {
            panic!("expected inline property schema");
        };
        assert!(name_schema.schema_data.nullable);
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("response schemas")
                && warning.contains("tolerant deserialization")));
    }

    #[test]
    fn response_only_component_schema_refs_are_relaxed() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r##"
openapi: 3.0.0
info:
  title: Relax Component Responses
  version: "1.0.0"
paths:
  /pets:
    get:
      operationId: getPet
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Pet"
components:
  schemas:
    Named:
      type: object
      required: [name]
      properties:
        name:
          type: string
    Pet:
      type: object
      required: [name, ability]
      properties:
        name:
          type: string
        ability:
          $ref: "#/components/schemas/Named"
"##,
        )
        .unwrap();

        normalize_unchecked_for_tests(&mut spec, &BackendCapabilities::progenitor()).unwrap();
        let components = spec.components.unwrap();
        let ReferenceOr::Item(schema) = components.schemas.get("Pet").unwrap() else {
            panic!("expected inline schema");
        };
        let SchemaKind::Type(Type::Object(object)) = &schema.schema_kind else {
            panic!("expected object schema");
        };
        assert!(object.required.is_empty());
        let ReferenceOr::Item(ability) = object.properties.get("ability").unwrap() else {
            panic!("expected wrapped reference schema");
        };
        assert!(ability.schema_data.nullable);
        assert!(matches!(ability.schema_kind, SchemaKind::Any(_)));

        let ReferenceOr::Item(named) = components.schemas.get("Named").unwrap() else {
            panic!("expected nested component schema");
        };
        let SchemaKind::Type(Type::Object(named_object)) = &named.schema_kind else {
            panic!("expected nested object schema");
        };
        assert!(named_object.required.is_empty());
    }

    #[test]
    fn request_body_prefers_application_json_and_warns() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Multi Media
  version: "1.0.0"
paths:
  /pets:
    post:
      operationId: createPet
      requestBody:
        content:
          application/xml:
            schema:
              type: object
          application/json:
            schema:
              type: object
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();

        let warnings =
            normalize_unchecked_for_tests(&mut spec, &BackendCapabilities::progenitor()).unwrap();
        let path = spec.paths.paths.get("/pets").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };
        let request_body = path.post.as_ref().unwrap().request_body.as_ref().unwrap();
        let ReferenceOr::Item(request_body) = request_body else {
            panic!("expected inline request body");
        };

        assert_eq!(
            request_body.content.keys().cloned().collect::<Vec<_>>(),
            vec![JSON_MIME]
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("kept application/json"));
        assert!(warnings[0].contains("dropped application/xml"));
    }

    #[test]
    fn optional_object_query_parameter_is_dropped_and_warns_once() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r##"
openapi: 3.0.0
info:
  title: Object Query
  version: "1.0.0"
paths:
  /search:
    get:
      operationId: searchPets
      parameters:
        - name: filter
          in: query
          schema:
            $ref: "#/components/schemas/Filter"
        - name: required_filter
          in: query
          required: true
          schema:
            $ref: "#/components/schemas/Filter"
      responses:
        '200':
          description: ok
components:
  schemas:
    Filter:
      type: object
      properties:
        color:
          type: string
"##,
        )
        .unwrap();

        let warnings =
            normalize_unchecked_for_tests(&mut spec, &BackendCapabilities::progenitor()).unwrap();
        let path = spec.paths.paths.get("/search").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };
        let operation = path.get.as_ref().unwrap();

        assert_eq!(operation.parameters.len(), 1);
        let ReferenceOr::Item(openapiv3::Parameter::Query { parameter_data, .. }) =
            operation.parameters.first().unwrap()
        else {
            panic!("expected inline query parameter");
        };
        assert_eq!(parameter_data.name, "required_filter");
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0],
            "dropped 1 optional object query parameters with progenitor-unsupported builder shape: searchPets.filter"
        );
    }

    #[test]
    fn deep_object_query_style_is_replaced_with_form_and_warns_once() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Deep Object Query
  version: "1.0.0"
paths:
  /search:
    get:
      operationId: searchPets
      parameters:
        - name: filter
          in: query
          required: true
          style: deepObject
          schema:
            type: object
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();

        let warnings =
            normalize_unchecked_for_tests(&mut spec, &BackendCapabilities::progenitor()).unwrap();
        let path = spec.paths.paths.get("/search").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };
        let ReferenceOr::Item(openapiv3::Parameter::Query { style, .. }) =
            path.get.as_ref().unwrap().parameters.first().unwrap()
        else {
            panic!("expected inline query parameter");
        };

        assert_eq!(*style, QueryStyle::Form);
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0],
            "normalized 1 query parameters — replaced unsupported deepObject style with form: searchPets.filter"
        );
    }

    #[test]
    fn unsupported_only_request_body_drops_operation_and_warns_once() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Multipart Body
  version: "1.0.0"
paths:
  /files:
    post:
      operationId: uploadFile
      requestBody:
        content:
          multipart/form-data:
            schema:
              type: object
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();

        let warnings =
            normalize_unchecked_for_tests(&mut spec, &BackendCapabilities::progenitor()).unwrap();
        let path = spec.paths.paths.get("/files").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };

        assert!(path.post.is_none());
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0],
            "dropped 1 operations with progenitor-unsupported request body: uploadFile"
        );
    }

    #[test]
    fn approved_component_unsupported_request_body_ref_drops_operation_and_component() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r##"
openapi: 3.0.0
info:
  title: Component Multipart Body
  version: "1.0.0"
paths:
  /files:
    post:
      operationId: uploadFile
      requestBody:
        $ref: "#/components/requestBodies/Upload"
      responses:
        '200':
          description: ok
components:
  requestBodies:
    Upload:
      content:
        multipart/form-data:
          schema:
            type: object
"##,
        )
        .unwrap();

        let warnings =
            normalize_unchecked_for_tests(&mut spec, &BackendCapabilities::progenitor()).unwrap();
        let path = spec.paths.paths.get("/files").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };

        assert!(path.post.is_none());
        assert!(!spec
            .components
            .as_ref()
            .unwrap()
            .request_bodies
            .contains_key("Upload"));
        assert!(warnings.iter().any(|warning| warning.contains(
            "normalized component requestBody Upload — dropped requestBody with only unsupported content types: multipart/form-data"
        )));
        assert!(warnings.iter().any(|warning| {
            warning.contains(
                "dropped 1 operations with progenitor-unsupported request body: uploadFile",
            )
        }));
    }

    #[test]
    fn request_body_keeps_supported_form_media_type() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Form Body
  version: "1.0.0"
paths:
  /tokens:
    post:
      operationId: createToken
      requestBody:
        content:
          application/x-www-form-urlencoded:
            schema:
              type: object
          multipart/form-data:
            schema:
              type: object
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();

        let warnings =
            normalize_unchecked_for_tests(&mut spec, &BackendCapabilities::progenitor()).unwrap();
        let path = spec.paths.paths.get("/tokens").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };
        let request_body = path.post.as_ref().unwrap().request_body.as_ref().unwrap();
        let ReferenceOr::Item(request_body) = request_body else {
            panic!("expected inline request body");
        };

        assert_eq!(
            request_body.content.keys().cloned().collect::<Vec<_>>(),
            vec![FORM_MIME]
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("kept application/x-www-form-urlencoded"));
        assert!(warnings[0].contains("dropped multipart/form-data"));
    }

    #[test]
    fn request_body_does_not_prefer_json_when_backend_does_not_support_json() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Form Only Body
  version: "1.0.0"
paths:
  /tokens:
    post:
      operationId: createToken
      requestBody:
        content:
          application/json:
            schema:
              type: object
          application/x-www-form-urlencoded:
            schema:
              type: object
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();
        let mut capabilities = BackendCapabilities::progenitor();
        capabilities.supported_request_body_content_types = &[FORM_MIME];

        let warnings = normalize_unchecked_for_tests(&mut spec, &capabilities).unwrap();
        let path = spec.paths.paths.get("/tokens").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };
        let request_body = path.post.as_ref().unwrap().request_body.as_ref().unwrap();
        let ReferenceOr::Item(request_body) = request_body else {
            panic!("expected inline request body");
        };

        assert_eq!(
            request_body.content.keys().cloned().collect::<Vec<_>>(),
            vec![FORM_MIME]
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("kept application/x-www-form-urlencoded"));
        assert!(warnings[0].contains("dropped application/json"));
    }

    #[test]
    fn schemaless_request_body_is_dropped_and_warns() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Schemaless Body
  version: "1.0.0"
paths:
  /pets:
    post:
      operationId: createPet
      requestBody:
        content:
          application/json: {}
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();

        let warnings =
            normalize_unchecked_for_tests(&mut spec, &BackendCapabilities::progenitor()).unwrap();
        let path = spec.paths.paths.get("/pets").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };

        assert!(path.post.as_ref().unwrap().request_body.is_none());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("dropped requestBody (no schema specified)"));
    }
}
