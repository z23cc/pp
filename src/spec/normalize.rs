use anyhow::Result;
use openapiv3::OpenAPI;

use super::normalization_rules::{self as rules, typed};
use super::report::ReportEntry;

mod operation_naming;
mod progenitor_compatibility;
mod response_relaxation;

#[cfg(test)]
use openapiv3::{QueryStyle, ReferenceOr, SchemaKind, StatusCode, Type};
#[cfg(test)]
use progenitor_compatibility::{FORM_MIME, JSON_MIME};

pub fn normalize(spec: &mut OpenAPI) -> Result<Vec<ReportEntry>> {
    let mut reports = Vec::new();
    apply_operation_naming_rules(spec, &mut reports);
    let compatibility_stats = progenitor_compatibility::apply(spec, &mut reports)?;
    progenitor_compatibility::emit_summary_reports(&mut reports, &compatibility_stats);
    apply_response_relaxation_rules(spec, &mut reports);
    progenitor_compatibility::emit_optional_object_query_param_report(
        &mut reports,
        &compatibility_stats,
    );

    Ok(reports)
}

fn apply_operation_naming_rules(spec: &mut OpenAPI, reports: &mut Vec<ReportEntry>) {
    operation_naming::apply(spec, reports);
}

fn apply_response_relaxation_rules(spec: &mut OpenAPI, reports: &mut Vec<ReportEntry>) {
    let relaxed_responses = response_relaxation::relax_response_schemas(spec);
    if relaxed_responses > 0 {
        reports.push(rules::typed_warning(
            typed::RESPONSE_SCHEMAS_RELAXED,
            format!("normalized {relaxed_responses} response schemas — relaxed output fields for tolerant deserialization"),
            None,
        ));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::report::{ReportStage, ReportSubject};

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

        let warnings = normalize(&mut spec).unwrap();
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

        let warnings = normalize(&mut spec).unwrap();
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

        let warnings = normalize(&mut spec).unwrap();
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

        let warnings = normalize(&mut spec).unwrap();
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

        let warnings = normalize(&mut spec).unwrap();
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

        normalize(&mut spec).unwrap();
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

        let warnings = normalize(&mut spec).unwrap();
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

        let warnings = normalize(&mut spec).unwrap();
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

        let warnings = normalize(&mut spec).unwrap();
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

        let warnings = normalize(&mut spec).unwrap();
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

        let warnings = normalize(&mut spec).unwrap();
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

        let warnings = normalize(&mut spec).unwrap();
        let path = spec.paths.paths.get("/pets").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };

        assert!(path.post.as_ref().unwrap().request_body.is_none());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("dropped requestBody (no schema specified)"));
    }
}
