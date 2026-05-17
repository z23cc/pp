use openapiv3::{
    AdditionalProperties, Header, MediaType, OpenAPI, Operation, Parameter, ParameterData,
    ParameterSchemaOrContent, ReferenceOr, RequestBody, Response, Schema, SchemaKind, Type,
};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComponentKind {
    Schema,
    Response,
    Parameter,
    RequestBody,
    Header,
    SecurityScheme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorkItem {
    kind: ComponentKind,
    name_index: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ComponentRefs {
    pub(crate) schemas: BTreeSet<String>,
    pub(crate) responses: BTreeSet<String>,
    pub(crate) parameters: BTreeSet<String>,
    pub(crate) request_bodies: BTreeSet<String>,
    pub(crate) headers: BTreeSet<String>,
    pub(crate) security_schemes: BTreeSet<String>,
    names: Vec<String>,
}

pub(crate) fn collect_reachable_components(api: &OpenAPI) -> ComponentRefs {
    let mut refs = ComponentRefs::default();
    let mut worklist = Vec::new();
    let mut inherits_root_security = false;

    for path_item in api.paths.paths.values() {
        let ReferenceOr::Item(item) = path_item else {
            continue;
        };
        for parameter in &item.parameters {
            scan_parameter_ref(parameter, &mut refs, &mut worklist);
        }
        for (_method, operation) in item.iter() {
            scan_operation(operation, &mut refs, &mut worklist);
            match &operation.security {
                Some(requirements) => {
                    scan_security_requirements(requirements, &mut refs, &mut worklist)
                }
                None => inherits_root_security = true,
            }
        }
    }

    if inherits_root_security {
        if let Some(requirements) = &api.security {
            scan_security_requirements(requirements, &mut refs, &mut worklist);
        }
    }

    while let Some(item) = worklist.pop() {
        let Some(components) = &api.components else {
            continue;
        };
        let name = refs.names[item.name_index].clone();
        match item.kind {
            ComponentKind::Schema => {
                if let Some(schema) = components.schemas.get(&name) {
                    scan_schema_ref(schema, &mut refs, &mut worklist);
                }
            }
            ComponentKind::Response => {
                if let Some(response) = components.responses.get(&name) {
                    scan_response_ref(response, &mut refs, &mut worklist);
                }
            }
            ComponentKind::Parameter => {
                if let Some(parameter) = components.parameters.get(&name) {
                    scan_parameter_ref(parameter, &mut refs, &mut worklist);
                }
            }
            ComponentKind::RequestBody => {
                if let Some(request_body) = components.request_bodies.get(&name) {
                    scan_request_body_ref(request_body, &mut refs, &mut worklist);
                }
            }
            ComponentKind::Header => {
                if let Some(header) = components.headers.get(&name) {
                    scan_header_ref(header, &mut refs, &mut worklist);
                }
            }
            ComponentKind::SecurityScheme => {}
        }
    }

    refs
}

fn scan_operation(operation: &Operation, refs: &mut ComponentRefs, worklist: &mut Vec<WorkItem>) {
    for parameter in &operation.parameters {
        scan_parameter_ref(parameter, refs, worklist);
    }
    if let Some(request_body) = &operation.request_body {
        scan_request_body_ref(request_body, refs, worklist);
    }
    scan_responses(&operation.responses, refs, worklist);
}

fn scan_responses(
    responses: &openapiv3::Responses,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    if let Some(response) = &responses.default {
        scan_response_ref(response, refs, worklist);
    }
    for response in responses.responses.values() {
        scan_response_ref(response, refs, worklist);
    }
}

fn scan_security_requirements(
    requirements: &[openapiv3::SecurityRequirement],
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    for requirement in requirements {
        for name in requirement.keys() {
            add_named_ref(ComponentKind::SecurityScheme, name, refs, worklist);
        }
    }
}

fn scan_parameter_ref(
    parameter: &ReferenceOr<Parameter>,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    match parameter {
        ReferenceOr::Reference { reference } => add_component_ref(reference, refs, worklist),
        ReferenceOr::Item(parameter) => scan_parameter(parameter, refs, worklist),
    }
}

fn scan_parameter(parameter: &Parameter, refs: &mut ComponentRefs, worklist: &mut Vec<WorkItem>) {
    scan_parameter_data(parameter.parameter_data_ref(), refs, worklist);
}

fn scan_parameter_data(
    data: &ParameterData,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    scan_parameter_format(&data.format, refs, worklist);
}

fn scan_parameter_format(
    format: &ParameterSchemaOrContent,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    match format {
        ParameterSchemaOrContent::Schema(schema) => scan_schema_ref(schema, refs, worklist),
        ParameterSchemaOrContent::Content(content) => scan_content(content, refs, worklist),
    }
}

fn scan_request_body_ref(
    request_body: &ReferenceOr<RequestBody>,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    match request_body {
        ReferenceOr::Reference { reference } => add_component_ref(reference, refs, worklist),
        ReferenceOr::Item(request_body) => scan_request_body(request_body, refs, worklist),
    }
}

fn scan_request_body(
    request_body: &RequestBody,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    scan_content(&request_body.content, refs, worklist);
}

fn scan_response_ref(
    response: &ReferenceOr<Response>,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    match response {
        ReferenceOr::Reference { reference } => add_component_ref(reference, refs, worklist),
        ReferenceOr::Item(response) => scan_response(response, refs, worklist),
    }
}

fn scan_response(response: &Response, refs: &mut ComponentRefs, worklist: &mut Vec<WorkItem>) {
    for header in response.headers.values() {
        scan_header_ref(header, refs, worklist);
    }
    scan_content(&response.content, refs, worklist);
}

fn scan_header_ref(
    header: &ReferenceOr<Header>,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    match header {
        ReferenceOr::Reference { reference } => add_component_ref(reference, refs, worklist),
        ReferenceOr::Item(header) => scan_header(header, refs, worklist),
    }
}

fn scan_header(header: &Header, refs: &mut ComponentRefs, worklist: &mut Vec<WorkItem>) {
    scan_parameter_format(&header.format, refs, worklist);
}

fn scan_content(
    content: &indexmap::IndexMap<String, MediaType>,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    for media_type in content.values() {
        scan_media_type(media_type, refs, worklist);
    }
}

fn scan_media_type(media_type: &MediaType, refs: &mut ComponentRefs, worklist: &mut Vec<WorkItem>) {
    if let Some(schema) = &media_type.schema {
        scan_schema_ref(schema, refs, worklist);
    }
    for encoding in media_type.encoding.values() {
        for header in encoding.headers.values() {
            scan_header_ref(header, refs, worklist);
        }
    }
}

fn scan_schema_ref(
    schema: &ReferenceOr<Schema>,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    match schema {
        ReferenceOr::Reference { reference } => add_component_ref(reference, refs, worklist),
        ReferenceOr::Item(schema) => scan_schema(schema, refs, worklist),
    }
}

fn scan_boxed_schema_ref(
    schema: &ReferenceOr<Box<Schema>>,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    match schema {
        ReferenceOr::Reference { reference } => add_component_ref(reference, refs, worklist),
        ReferenceOr::Item(schema) => scan_schema(schema, refs, worklist),
    }
}

fn scan_schema(schema: &Schema, refs: &mut ComponentRefs, worklist: &mut Vec<WorkItem>) {
    match &schema.schema_kind {
        SchemaKind::Type(typ) => scan_type(typ, refs, worklist),
        SchemaKind::OneOf { one_of } => scan_schema_refs(one_of, refs, worklist),
        SchemaKind::AllOf { all_of } => scan_schema_refs(all_of, refs, worklist),
        SchemaKind::AnyOf { any_of } => scan_schema_refs(any_of, refs, worklist),
        SchemaKind::Not { not } => scan_schema_ref(not, refs, worklist),
        SchemaKind::Any(any) => {
            for property in any.properties.values() {
                scan_boxed_schema_ref(property, refs, worklist);
            }
            if let Some(additional) = &any.additional_properties {
                scan_additional_properties(additional, refs, worklist);
            }
            if let Some(items) = &any.items {
                scan_boxed_schema_ref(items, refs, worklist);
            }
            scan_schema_refs(&any.one_of, refs, worklist);
            scan_schema_refs(&any.all_of, refs, worklist);
            scan_schema_refs(&any.any_of, refs, worklist);
            if let Some(not) = &any.not {
                scan_schema_ref(not, refs, worklist);
            }
        }
    }
}

fn scan_schema_refs(
    schemas: &[ReferenceOr<Schema>],
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    for schema in schemas {
        scan_schema_ref(schema, refs, worklist);
    }
}

fn scan_type(typ: &Type, refs: &mut ComponentRefs, worklist: &mut Vec<WorkItem>) {
    match typ {
        Type::Object(object) => {
            for property in object.properties.values() {
                scan_boxed_schema_ref(property, refs, worklist);
            }
            if let Some(additional) = &object.additional_properties {
                scan_additional_properties(additional, refs, worklist);
            }
        }
        Type::Array(array) => {
            if let Some(items) = &array.items {
                scan_boxed_schema_ref(items, refs, worklist);
            }
        }
        Type::String(_) | Type::Number(_) | Type::Integer(_) | Type::Boolean(_) => {}
    }
}

fn scan_additional_properties(
    additional: &AdditionalProperties,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    match additional {
        AdditionalProperties::Any(_) => {}
        AdditionalProperties::Schema(schema) => scan_schema_ref(schema, refs, worklist),
    }
}

fn add_component_ref(reference: &str, refs: &mut ComponentRefs, worklist: &mut Vec<WorkItem>) {
    let Some((kind, name)) = parse_component_ref(reference) else {
        return;
    };
    add_named_ref(kind, &name, refs, worklist);
}

fn add_named_ref(
    kind: ComponentKind,
    name: &str,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    let inserted = match kind {
        ComponentKind::Schema => refs.schemas.insert(name.to_string()),
        ComponentKind::Response => refs.responses.insert(name.to_string()),
        ComponentKind::Parameter => refs.parameters.insert(name.to_string()),
        ComponentKind::RequestBody => refs.request_bodies.insert(name.to_string()),
        ComponentKind::Header => refs.headers.insert(name.to_string()),
        ComponentKind::SecurityScheme => refs.security_schemes.insert(name.to_string()),
    };
    if inserted {
        refs.names.push(name.to_string());
        worklist.push(WorkItem {
            kind,
            name_index: refs.names.len() - 1,
        });
    }
}

pub(crate) fn parse_component_ref(reference: &str) -> Option<(ComponentKind, String)> {
    let rest = reference.strip_prefix("#/components/")?;
    let (category, raw_name) = rest.split_once('/')?;
    let kind = match category {
        "schemas" => ComponentKind::Schema,
        "responses" => ComponentKind::Response,
        "parameters" => ComponentKind::Parameter,
        "requestBodies" => ComponentKind::RequestBody,
        "headers" => ComponentKind::Header,
        "securitySchemes" => ComponentKind::SecurityScheme,
        _ => return None,
    };
    Some((kind, decode_json_pointer(raw_name)))
}

fn decode_json_pointer(input: &str) -> String {
    input.replace("~1", "/").replace("~0", "~")
}

#[cfg(test)]
pub(crate) fn collect_raw_schema_refs(schema: &ReferenceOr<Schema>, refs: &mut BTreeSet<String>) {
    match schema {
        ReferenceOr::Reference { reference } => {
            refs.insert(reference.clone());
        }
        ReferenceOr::Item(schema) => collect_raw_schema_refs_in_schema(schema, refs),
    }
}

#[cfg(test)]
pub(crate) fn collect_raw_boxed_schema_refs(
    schema: &ReferenceOr<Box<Schema>>,
    refs: &mut BTreeSet<String>,
) {
    match schema {
        ReferenceOr::Reference { reference } => {
            refs.insert(reference.clone());
        }
        ReferenceOr::Item(schema) => collect_raw_schema_refs_in_schema(schema, refs),
    }
}

#[cfg(test)]
pub(crate) fn collect_raw_schema_refs_in_schema(schema: &Schema, refs: &mut BTreeSet<String>) {
    match &schema.schema_kind {
        SchemaKind::Type(Type::Object(object)) => {
            for property in object.properties.values() {
                collect_raw_boxed_schema_refs(property, refs);
            }
            if let Some(AdditionalProperties::Schema(schema)) =
                object.additional_properties.as_ref()
            {
                collect_raw_schema_refs(schema.as_ref(), refs);
            }
        }
        SchemaKind::Type(Type::Array(array)) => {
            if let Some(items) = array.items.as_ref() {
                collect_raw_boxed_schema_refs(items, refs);
            }
        }
        SchemaKind::OneOf { one_of } => {
            for schema in one_of {
                collect_raw_schema_refs(schema, refs);
            }
        }
        SchemaKind::AllOf { all_of } => {
            for schema in all_of {
                collect_raw_schema_refs(schema, refs);
            }
        }
        SchemaKind::AnyOf { any_of } => {
            for schema in any_of {
                collect_raw_schema_refs(schema, refs);
            }
        }
        SchemaKind::Not { not } => collect_raw_schema_refs(not.as_ref(), refs),
        SchemaKind::Any(any) => {
            for property in any.properties.values() {
                collect_raw_boxed_schema_refs(property, refs);
            }
            if let Some(items) = any.items.as_ref() {
                collect_raw_boxed_schema_refs(items, refs);
            }
            if let Some(AdditionalProperties::Schema(schema)) = any.additional_properties.as_ref() {
                collect_raw_schema_refs(schema.as_ref(), refs);
            }
            for schema in &any.one_of {
                collect_raw_schema_refs(schema, refs);
            }
            for schema in &any.all_of {
                collect_raw_schema_refs(schema, refs);
            }
            for schema in &any.any_of {
                collect_raw_schema_refs(schema, refs);
            }
            if let Some(not) = any.not.as_ref() {
                collect_raw_schema_refs(not.as_ref(), refs);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openapiv3::OpenAPI;

    #[test]
    fn parses_and_decodes_component_refs() {
        assert_eq!(
            parse_component_ref("#/components/schemas/Foo~1Bar~0Baz"),
            Some((ComponentKind::Schema, "Foo/Bar~Baz".to_string()))
        );
        assert_eq!(parse_component_ref("#/paths/~1pets"), None);
        assert_eq!(parse_component_ref("#/components/examples/Foo"), None);
    }

    #[test]
    fn reachable_components_include_nested_schema_and_media_headers() {
        let api: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info: { title: Reference Test, version: '1.0' }
security:
  - RootKey: []
paths:
  /items:
    get:
      parameters:
        - $ref: '#/components/parameters/KeptParam'
      responses:
        '200':
          $ref: '#/components/responses/KeptResponse'
components:
  parameters:
    KeptParam:
      name: filter
      in: query
      schema:
        $ref: '#/components/schemas/Filter'
  responses:
    KeptResponse:
      description: ok
      headers:
        X-Next:
          $ref: '#/components/headers/NextHeader'
      content:
        application/json:
          schema:
            $ref: '#/components/schemas/Envelope'
          encoding:
            data:
              headers:
                X-Encoded:
                  $ref: '#/components/headers/EncodedHeader'
  headers:
    NextHeader:
      schema:
        $ref: '#/components/schemas/PageToken'
    EncodedHeader:
      schema:
        $ref: '#/components/schemas/EncodedToken'
  securitySchemes:
    RootKey:
      type: apiKey
      in: header
      name: X-Root-Key
  schemas:
    Envelope:
      type: object
      properties:
        data:
          $ref: '#/components/schemas/Item'
    Item:
      allOf:
        - $ref: '#/components/schemas/Base'
        - type: object
          properties:
            children:
              type: array
              items:
                $ref: '#/components/schemas/Child'
    Base:
      type: object
    Child:
      anyOf:
        - $ref: '#/components/schemas/Leaf'
    Leaf:
      not:
        $ref: '#/components/schemas/Never'
    Never:
      type: string
    Filter:
      type: object
    PageToken:
      type: string
    EncodedToken:
      type: string
    Dropped:
      type: object
"#,
        )
        .unwrap();

        let refs = collect_reachable_components(&api);
        for name in [
            "Envelope",
            "Item",
            "Base",
            "Child",
            "Leaf",
            "Never",
            "Filter",
            "PageToken",
            "EncodedToken",
        ] {
            assert!(refs.schemas.contains(name), "missing schema {name}");
        }
        assert!(refs.parameters.contains("KeptParam"));
        assert!(refs.responses.contains("KeptResponse"));
        assert!(refs.headers.contains("NextHeader"));
        assert!(refs.headers.contains("EncodedHeader"));
        assert!(refs.security_schemes.contains("RootKey"));
        assert!(!refs.schemas.contains("Dropped"));
    }

    #[test]
    fn raw_schema_refs_cover_composition_and_any_schema_fields() {
        let schema: ReferenceOr<Schema> = serde_yaml::from_str(
            r#"
type: object
properties:
  direct:
    $ref: '#/components/schemas/Direct'
additionalProperties:
  $ref: '#/components/schemas/Additional'
"#,
        )
        .unwrap();
        let mut refs = BTreeSet::new();
        collect_raw_schema_refs(&schema, &mut refs);
        assert!(refs.contains("#/components/schemas/Direct"));
        assert!(refs.contains("#/components/schemas/Additional"));
    }

    #[test]
    fn raw_schema_refs_cover_any_schema_composition_fields() {
        let mut any = openapiv3::AnySchema::default();
        any.one_of.push(ReferenceOr::Reference {
            reference: "#/components/schemas/One".to_string(),
        });
        any.all_of.push(ReferenceOr::Reference {
            reference: "#/components/schemas/All".to_string(),
        });
        any.any_of.push(ReferenceOr::Reference {
            reference: "#/components/schemas/Any".to_string(),
        });
        any.not = Some(Box::new(ReferenceOr::Reference {
            reference: "#/components/schemas/Not".to_string(),
        }));
        let schema = Schema {
            schema_data: Default::default(),
            schema_kind: SchemaKind::Any(any),
        };

        let mut refs = BTreeSet::new();
        collect_raw_schema_refs_in_schema(&schema, &mut refs);

        for reference in [
            "#/components/schemas/One",
            "#/components/schemas/All",
            "#/components/schemas/Any",
            "#/components/schemas/Not",
        ] {
            assert!(refs.contains(reference), "missing {reference}");
        }
    }
}
