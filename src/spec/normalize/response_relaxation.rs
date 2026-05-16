use openapiv3::{MediaType, OpenAPI, ReferenceOr, Response, Schema, SchemaData, SchemaKind, Type};
use std::collections::BTreeSet;

use crate::backend::BackendCapabilities;
use crate::spec::normalization_rules::{self as rules, typed};
use crate::spec::references;
use crate::spec::report::ReportEntry;
use crate::spec::transform::{TransformActionKind, TransformAuditEntry};
use crate::spec::traversal;
use serde_json::json;

type ReplaceCount = usize;

#[derive(Debug, Clone, Default)]
pub(super) struct ResponseRelaxationPlan {
    action: Option<RelaxResponseSchemas>,
}

impl ResponseRelaxationPlan {
    pub(super) fn report_entries(&self) -> Vec<ReportEntry> {
        self.action
            .iter()
            .map(|action| action.report.clone())
            .collect()
    }

    pub(super) fn audit_entries(&self) -> Vec<TransformAuditEntry> {
        self.action
            .iter()
            .map(RelaxResponseSchemas::audit_entry)
            .collect()
    }
}

#[derive(Debug, Clone)]
struct RelaxResponseSchemas {
    count: ReplaceCount,
    report: ReportEntry,
}

impl RelaxResponseSchemas {
    fn audit_entry(&self) -> TransformAuditEntry {
        TransformAuditEntry::new(
            "typed_normalization",
            self.report.code,
            "responses.*.schema",
            format!(
                "relax {} response schemas for tolerant deserialization",
                self.count
            ),
        )
        .with_action_kind(TransformActionKind::Relax)
        .with_backend_requirement_id("progenitor.response_deserialization_tolerance")
        .with_backend_requirement(
            "backend requires response-only schemas to deserialize missing/null fields tolerantly",
        )
        .with_before_after(
            "required response fields / strict property schemas",
            "optional nullable response fields",
        )
        .with_before_after_json(
            json!({ "required_fields": "strict", "property_schemas": "typed" }),
            json!({ "required_fields": "optional", "property_schemas": "nullable_any" }),
        )
    }
}

pub(super) fn propose(
    spec: &OpenAPI,
    backend_capabilities: &BackendCapabilities,
) -> ResponseRelaxationPlan {
    if !backend_capabilities.responses.requires_relaxed_schemas {
        return ResponseRelaxationPlan::default();
    }

    let mut candidate = spec.clone();
    let count = relax_response_schemas(&mut candidate);
    if count == 0 {
        return ResponseRelaxationPlan::default();
    }

    ResponseRelaxationPlan {
        action: Some(RelaxResponseSchemas {
            count,
            report: response_schemas_relaxed_report(count),
        }),
    }
}

pub(super) fn apply_approved(
    spec: &mut OpenAPI,
    reports: &mut Vec<ReportEntry>,
    approved_plan: &ResponseRelaxationPlan,
) {
    let Some(action) = &approved_plan.action else {
        return;
    };

    let relaxed = relax_response_schemas(spec);
    debug_assert_eq!(
        relaxed, action.count,
        "approved response relaxation count drifted between proposal and apply"
    );
    reports.push(action.report.clone());
}

fn response_schemas_relaxed_report(count: ReplaceCount) -> ReportEntry {
    rules::typed_warning(
        typed::RESPONSE_SCHEMAS_RELAXED,
        format!("normalized {count} response schemas — relaxed output fields for tolerant deserialization"),
        None,
    )
}

pub(super) fn relax_response_schemas(spec: &mut OpenAPI) -> ReplaceCount {
    let request_refs = collect_request_schema_refs(spec);
    let response_refs = collect_response_schema_refs(spec);
    let mut count = 0;

    count += relax_inline_response_schemas(spec);

    let Some(components) = spec.components.as_mut() else {
        return count;
    };
    for reference in response_refs.difference(&request_refs) {
        let Some(name) = schema_component_name(reference) else {
            continue;
        };
        let Some(ReferenceOr::Item(schema)) = components.schemas.get_mut(name) else {
            continue;
        };
        count += relax_schema_for_response(schema);
    }

    count
}

fn collect_request_schema_refs(spec: &OpenAPI) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    for operation_ref in traversal::operations(spec) {
        for parameter in operation_ref
            .path_parameters
            .iter()
            .chain(operation_ref.operation.parameters.iter())
        {
            collect_parameter_schema_refs(parameter, &mut refs);
        }
        if let Some(ReferenceOr::Item(request_body)) = &operation_ref.operation.request_body {
            collect_content_schema_refs(&request_body.content, &mut refs);
        }
    }
    if let Some(components) = spec.components.as_ref() {
        for parameter in components.parameters.values() {
            collect_parameter_schema_refs(parameter, &mut refs);
        }
        for request_body in components.request_bodies.values() {
            if let ReferenceOr::Item(request_body) = request_body {
                collect_content_schema_refs(&request_body.content, &mut refs);
            }
        }
    }
    expand_component_schema_refs(spec, &mut refs);
    refs
}

fn collect_parameter_schema_refs(
    parameter: &ReferenceOr<openapiv3::Parameter>,
    refs: &mut BTreeSet<String>,
) {
    let ReferenceOr::Item(parameter) = parameter else {
        return;
    };
    let data = match parameter {
        openapiv3::Parameter::Query { parameter_data, .. }
        | openapiv3::Parameter::Header { parameter_data, .. }
        | openapiv3::Parameter::Path { parameter_data, .. }
        | openapiv3::Parameter::Cookie { parameter_data, .. } => parameter_data,
    };
    if let openapiv3::ParameterSchemaOrContent::Schema(schema) = &data.format {
        references::collect_raw_schema_refs(schema, refs);
    }
}

fn collect_response_schema_refs(spec: &OpenAPI) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    for response in responses(spec) {
        collect_content_schema_refs(&response.content, &mut refs);
    }
    expand_component_schema_refs(spec, &mut refs);
    refs
}

fn expand_component_schema_refs(spec: &OpenAPI, refs: &mut BTreeSet<String>) {
    let Some(components) = spec.components.as_ref() else {
        return;
    };
    let mut queue: Vec<String> = refs.iter().cloned().collect();
    while let Some(reference) = queue.pop() {
        let Some(name) = schema_component_name(&reference) else {
            continue;
        };
        let Some(ReferenceOr::Item(schema)) = components.schemas.get(name) else {
            continue;
        };
        let before = refs.len();
        references::collect_raw_schema_refs_in_schema(schema, refs);
        if refs.len() > before {
            queue = refs.iter().cloned().collect();
        }
    }
}

fn collect_content_schema_refs(
    content: &indexmap::IndexMap<String, MediaType>,
    refs: &mut BTreeSet<String>,
) {
    for media_type in content.values() {
        if let Some(schema) = media_type.schema.as_ref() {
            references::collect_raw_schema_refs(schema, refs);
        }
    }
}

fn responses(spec: &OpenAPI) -> Vec<&Response> {
    let mut responses = Vec::new();
    if let Some(components) = spec.components.as_ref() {
        for response in components.responses.values() {
            if let ReferenceOr::Item(response) = response {
                responses.push(response);
            }
        }
    }
    for operation_ref in traversal::operations(spec) {
        for response in operation_ref.operation.responses.responses.values() {
            if let ReferenceOr::Item(response) = response {
                responses.push(response);
            }
        }
        if let Some(ReferenceOr::Item(response)) =
            operation_ref.operation.responses.default.as_ref()
        {
            responses.push(response);
        }
    }
    responses
}

fn relax_inline_response_schemas(spec: &mut OpenAPI) -> ReplaceCount {
    let mut count = 0;
    if let Some(components) = spec.components.as_mut() {
        for response in components.responses.values_mut() {
            if let ReferenceOr::Item(response) = response {
                count += relax_response(response);
            }
        }
    }
    traversal::visit_operations_mut(spec, |operation_ref| {
        for response in operation_ref.operation.responses.responses.values_mut() {
            if let ReferenceOr::Item(response) = response {
                count += relax_response(response);
            }
        }
        if let Some(ReferenceOr::Item(response)) =
            operation_ref.operation.responses.default.as_mut()
        {
            count += relax_response(response);
        }
    });
    count
}

fn schema_component_name(reference: &str) -> Option<&str> {
    reference.strip_prefix("#/components/schemas/")
}

fn relax_response(response: &mut Response) -> ReplaceCount {
    response
        .content
        .values_mut()
        .filter_map(|media_type| media_type.schema.as_mut())
        .map(relax_schema_ref_for_response)
        .sum()
}

fn relax_schema_ref_for_response(schema: &mut ReferenceOr<Schema>) -> ReplaceCount {
    match schema {
        ReferenceOr::Item(schema) => relax_schema_for_response(schema),
        ReferenceOr::Reference { .. } => 0,
    }
}

fn relax_boxed_schema_ref_for_response(schema: &mut ReferenceOr<Box<Schema>>) -> ReplaceCount {
    match schema {
        ReferenceOr::Item(schema) => relax_schema_for_response(schema.as_mut()),
        ReferenceOr::Reference { .. } => 0,
    }
}

fn relax_schema_for_response(schema: &mut Schema) -> ReplaceCount {
    let mut count = 0;
    match &mut schema.schema_kind {
        SchemaKind::Type(Type::Object(object)) => {
            count += relax_object_for_response(object);
        }
        SchemaKind::Type(Type::Array(array)) => {
            if let Some(items) = array.items.as_mut() {
                count += relax_boxed_schema_ref_for_response(items);
            }
        }
        SchemaKind::OneOf { one_of } => {
            count += one_of
                .iter_mut()
                .map(relax_schema_ref_for_response)
                .sum::<usize>();
        }
        SchemaKind::AllOf { all_of } => {
            count += all_of
                .iter_mut()
                .map(relax_schema_ref_for_response)
                .sum::<usize>();
        }
        SchemaKind::AnyOf { any_of } => {
            count += any_of
                .iter_mut()
                .map(relax_schema_ref_for_response)
                .sum::<usize>();
        }
        SchemaKind::Not { not } => {
            count += relax_schema_ref_for_response(not.as_mut());
        }
        SchemaKind::Any(any) => {
            let had_shape = !any.properties.is_empty() || !any.required.is_empty();
            if !any.required.is_empty() {
                any.required.clear();
            }
            for property in any.properties.values_mut() {
                count += relax_property_for_response(property);
            }
            if let Some(items) = any.items.as_mut() {
                count += relax_boxed_schema_ref_for_response(items);
            }
            if let Some(openapiv3::AdditionalProperties::Schema(schema)) =
                any.additional_properties.as_mut()
            {
                count += relax_schema_ref_for_response(schema.as_mut());
            }
            if had_shape {
                count += 1;
            }
        }
        _ => {}
    }
    count
}

fn relax_object_for_response(object: &mut openapiv3::ObjectType) -> ReplaceCount {
    let had_shape = !object.properties.is_empty() || !object.required.is_empty();
    if !object.required.is_empty() {
        object.required.clear();
    }
    let mut count = 0;
    for property in object.properties.values_mut() {
        count += relax_property_for_response(property);
    }
    if let Some(openapiv3::AdditionalProperties::Schema(schema)) =
        object.additional_properties.as_mut()
    {
        count += relax_schema_ref_for_response(schema.as_mut());
    }
    if had_shape {
        count += 1;
    }
    count
}

fn relax_property_for_response(property: &mut ReferenceOr<Box<Schema>>) -> ReplaceCount {
    match property {
        ReferenceOr::Item(schema) => {
            schema.schema_data.nullable = true;
            schema.schema_kind = SchemaKind::Any(Default::default());
        }
        ReferenceOr::Reference { .. } => {
            *property = ReferenceOr::Item(Box::new(Schema {
                schema_data: SchemaData {
                    nullable: true,
                    ..SchemaData::default()
                },
                schema_kind: SchemaKind::Any(Default::default()),
            }));
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_level_parameter_refs_keep_shared_schema_request_side() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Path Param Relaxation
  version: "1.0.0"
paths:
  /search:
    parameters:
      - in: query
        name: filter
        required: true
        schema:
          $ref: '#/components/schemas/Filter'
    get:
      operationId: search
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Filter'
components:
  schemas:
    Filter:
      type: object
      required: [term]
      properties:
        term:
          type: string
"#,
        )
        .unwrap();

        assert_eq!(relax_response_schemas(&mut spec), 0);
        let components = spec.components.unwrap();
        let ReferenceOr::Item(schema) = components.schemas.get("Filter").unwrap() else {
            panic!("expected inline schema");
        };
        let SchemaKind::Type(Type::Object(object)) = &schema.schema_kind else {
            panic!("expected object schema");
        };
        assert_eq!(object.required, vec!["term"]);
    }

    #[test]
    fn component_parameter_refs_keep_shared_schema_request_side() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Component Param Relaxation
  version: "1.0.0"
paths:
  /search:
    get:
      operationId: search
      parameters:
        - $ref: '#/components/parameters/FilterParam'
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Filter'
components:
  parameters:
    FilterParam:
      in: query
      name: filter
      required: true
      schema:
        $ref: '#/components/schemas/Filter'
  schemas:
    Filter:
      type: object
      required: [term]
      properties:
        term:
          type: string
"#,
        )
        .unwrap();

        assert_eq!(relax_response_schemas(&mut spec), 0);
        let components = spec.components.unwrap();
        let ReferenceOr::Item(schema) = components.schemas.get("Filter").unwrap() else {
            panic!("expected inline schema");
        };
        let SchemaKind::Type(Type::Object(object)) = &schema.schema_kind else {
            panic!("expected object schema");
        };
        assert_eq!(object.required, vec!["term"]);
    }
}
