use anyhow::{bail, Result};
use openapiv3::{
    AdditionalProperties, Header, MediaType, OpenAPI, Operation, Parameter, ParameterData,
    ParameterSchemaOrContent, PathItem, ReferenceOr, RequestBody, Response, Schema, SchemaKind,
    Type,
};
use serde::Serialize;
use std::collections::BTreeSet;

use super::normalization_rules::{self as rules, slicing};
use super::report::{ReportEntry, ReportSubject};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SliceOptions {
    pub include_operations: Vec<String>,
    pub include_tags: Vec<String>,
    pub include_path_prefixes: Vec<String>,
    pub exclude_operations: Vec<String>,
}

impl SliceOptions {
    pub fn is_noop(&self) -> bool {
        self.include_operations.is_empty()
            && self.include_tags.is_empty()
            && self.include_path_prefixes.is_empty()
            && self.exclude_operations.is_empty()
    }

    fn has_includes(&self) -> bool {
        !self.include_operations.is_empty()
            || !self.include_tags.is_empty()
            || !self.include_path_prefixes.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OperationListing {
    pub id: String,
    pub method: String,
    pub path: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SliceReport {
    pub kept_operations: usize,
    pub dropped_operations: usize,
    pub pruned_components: PrunedComponents,
}

impl SliceReport {
    pub fn report_entries(&self) -> Vec<ReportEntry> {
        vec![
            rules::slicing_warning(
                slicing::OPERATIONS_FILTERED,
                format!(
                    "sliced spec — kept {} operations, dropped {} operations",
                    self.kept_operations, self.dropped_operations
                ),
                None,
            ),
            rules::slicing_warning(
                slicing::COMPONENTS_PRUNED,
                format!(
                    "pruned components — schemas {} -> {}, responses {} -> {}, parameters {} -> {}, requestBodies {} -> {}, headers {} -> {}, securitySchemes {} -> {}",
                    self.pruned_components.schemas_before,
                    self.pruned_components.schemas_after,
                    self.pruned_components.responses_before,
                    self.pruned_components.responses_after,
                    self.pruned_components.parameters_before,
                    self.pruned_components.parameters_after,
                    self.pruned_components.request_bodies_before,
                    self.pruned_components.request_bodies_after,
                    self.pruned_components.headers_before,
                    self.pruned_components.headers_after,
                    self.pruned_components.security_schemes_before,
                    self.pruned_components.security_schemes_after,
                ),
                Some(ReportSubject::component("components")),
            ),
        ]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrunedComponents {
    pub schemas_before: usize,
    pub schemas_after: usize,
    pub responses_before: usize,
    pub responses_after: usize,
    pub parameters_before: usize,
    pub parameters_after: usize,
    pub request_bodies_before: usize,
    pub request_bodies_after: usize,
    pub headers_before: usize,
    pub headers_after: usize,
    pub security_schemes_before: usize,
    pub security_schemes_after: usize,
}

pub fn list_operations(api: &OpenAPI) -> Vec<OperationListing> {
    let mut out = Vec::new();
    for (path, path_item) in api.paths.iter() {
        let ReferenceOr::Item(item) = path_item else {
            continue;
        };
        for (method, operation) in item.iter() {
            out.push(OperationListing {
                id: operation_identifier(method, path, operation),
                method: method.to_string(),
                path: path.clone(),
                tags: operation.tags.clone(),
            });
        }
    }
    out
}

pub fn slice_openapi(api: &mut OpenAPI, options: &SliceOptions) -> Result<SliceReport> {
    if options.is_noop() {
        return Ok(SliceReport::default());
    }

    let mut kept_operations = 0;
    let mut dropped_operations = 0;
    let has_includes = options.has_includes();

    for (path, path_item) in api.paths.paths.iter_mut() {
        let ReferenceOr::Item(item) = path_item else {
            if has_includes {
                dropped_operations += 1;
            }
            continue;
        };

        filter_operation_slot(
            "get",
            path,
            &mut item.get,
            options,
            &mut kept_operations,
            &mut dropped_operations,
        );
        filter_operation_slot(
            "put",
            path,
            &mut item.put,
            options,
            &mut kept_operations,
            &mut dropped_operations,
        );
        filter_operation_slot(
            "post",
            path,
            &mut item.post,
            options,
            &mut kept_operations,
            &mut dropped_operations,
        );
        filter_operation_slot(
            "delete",
            path,
            &mut item.delete,
            options,
            &mut kept_operations,
            &mut dropped_operations,
        );
        filter_operation_slot(
            "options",
            path,
            &mut item.options,
            options,
            &mut kept_operations,
            &mut dropped_operations,
        );
        filter_operation_slot(
            "head",
            path,
            &mut item.head,
            options,
            &mut kept_operations,
            &mut dropped_operations,
        );
        filter_operation_slot(
            "patch",
            path,
            &mut item.patch,
            options,
            &mut kept_operations,
            &mut dropped_operations,
        );
        filter_operation_slot(
            "trace",
            path,
            &mut item.trace,
            options,
            &mut kept_operations,
            &mut dropped_operations,
        );
    }

    api.paths.paths.retain(|_, path_item| match path_item {
        ReferenceOr::Item(item) => path_item_has_operations(item),
        ReferenceOr::Reference { .. } => !has_includes,
    });

    if kept_operations == 0 {
        bail!(
            "no operations matched slice filters; use `pp inspect --list-operations` to discover operation IDs/tags"
        );
    }

    drop_codegen_ignored_graph_roots(api);
    let refs = collect_reachable_components(api);
    let pruned_components = prune_components(api, &refs);

    Ok(SliceReport {
        kept_operations,
        dropped_operations,
        pruned_components,
    })
}

fn filter_operation_slot(
    method: &str,
    path: &str,
    operation: &mut Option<Operation>,
    options: &SliceOptions,
    kept: &mut usize,
    dropped: &mut usize,
) {
    let Some(op) = operation.as_ref() else {
        return;
    };

    if operation_matches(method, path, op, options) {
        *kept += 1;
    } else {
        *operation = None;
        *dropped += 1;
    }
}

fn operation_matches(
    method: &str,
    path: &str,
    operation: &Operation,
    options: &SliceOptions,
) -> bool {
    let id = operation_identifier(method, path, operation);
    if options
        .exclude_operations
        .iter()
        .any(|candidate| candidate == &id)
    {
        return false;
    }

    if !options.has_includes() {
        return true;
    }

    options
        .include_operations
        .iter()
        .any(|candidate| candidate == &id)
        || options
            .include_tags
            .iter()
            .any(|tag| operation.tags.iter().any(|op_tag| op_tag == tag))
        || options
            .include_path_prefixes
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

fn operation_identifier(method: &str, path: &str, operation: &Operation) -> String {
    operation
        .operation_id
        .clone()
        .unwrap_or_else(|| format!("{method} {path}"))
}

fn path_item_has_operations(item: &PathItem) -> bool {
    item.get.is_some()
        || item.put.is_some()
        || item.post.is_some()
        || item.delete.is_some()
        || item.options.is_some()
        || item.head.is_some()
        || item.patch.is_some()
        || item.trace.is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComponentKind {
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

#[derive(Default)]
struct Refs {
    schemas: BTreeSet<String>,
    responses: BTreeSet<String>,
    parameters: BTreeSet<String>,
    request_bodies: BTreeSet<String>,
    headers: BTreeSet<String>,
    security_schemes: BTreeSet<String>,
    names: Vec<String>,
}

fn drop_codegen_ignored_graph_roots(api: &mut OpenAPI) {
    for path_item in api.paths.paths.values_mut() {
        let ReferenceOr::Item(item) = path_item else {
            continue;
        };
        for operation in operations_mut(item) {
            operation.callbacks.clear();
            if let Some(response) = operation.responses.default.as_mut() {
                clear_response_links(response);
            }
            for response in operation.responses.responses.values_mut() {
                clear_response_links(response);
            }
        }
    }

    if let Some(components) = api.components.as_mut() {
        components.examples.clear();
        components.links.clear();
        components.callbacks.clear();
        for response in components.responses.values_mut() {
            clear_response_links(response);
        }
    }
}

fn clear_response_links(response: &mut ReferenceOr<Response>) {
    if let ReferenceOr::Item(response) = response {
        response.links.clear();
    }
}

fn operations_mut(item: &mut PathItem) -> Vec<&mut Operation> {
    let mut operations = Vec::new();
    if let Some(operation) = item.get.as_mut() {
        operations.push(operation);
    }
    if let Some(operation) = item.put.as_mut() {
        operations.push(operation);
    }
    if let Some(operation) = item.post.as_mut() {
        operations.push(operation);
    }
    if let Some(operation) = item.delete.as_mut() {
        operations.push(operation);
    }
    if let Some(operation) = item.options.as_mut() {
        operations.push(operation);
    }
    if let Some(operation) = item.head.as_mut() {
        operations.push(operation);
    }
    if let Some(operation) = item.patch.as_mut() {
        operations.push(operation);
    }
    if let Some(operation) = item.trace.as_mut() {
        operations.push(operation);
    }
    operations
}

fn collect_reachable_components(api: &OpenAPI) -> Refs {
    let mut refs = Refs::default();
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

fn scan_operation(operation: &Operation, refs: &mut Refs, worklist: &mut Vec<WorkItem>) {
    for parameter in &operation.parameters {
        scan_parameter_ref(parameter, refs, worklist);
    }
    if let Some(request_body) = &operation.request_body {
        scan_request_body_ref(request_body, refs, worklist);
    }
    scan_responses(&operation.responses, refs, worklist);
}

fn scan_responses(responses: &openapiv3::Responses, refs: &mut Refs, worklist: &mut Vec<WorkItem>) {
    if let Some(response) = &responses.default {
        scan_response_ref(response, refs, worklist);
    }
    for response in responses.responses.values() {
        scan_response_ref(response, refs, worklist);
    }
}

fn scan_security_requirements(
    requirements: &[openapiv3::SecurityRequirement],
    refs: &mut Refs,
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
    refs: &mut Refs,
    worklist: &mut Vec<WorkItem>,
) {
    match parameter {
        ReferenceOr::Reference { reference } => add_component_ref(reference, refs, worklist),
        ReferenceOr::Item(parameter) => scan_parameter(parameter, refs, worklist),
    }
}

fn scan_parameter(parameter: &Parameter, refs: &mut Refs, worklist: &mut Vec<WorkItem>) {
    scan_parameter_data(parameter.parameter_data_ref(), refs, worklist);
}

fn scan_parameter_data(data: &ParameterData, refs: &mut Refs, worklist: &mut Vec<WorkItem>) {
    scan_parameter_format(&data.format, refs, worklist);
}

fn scan_parameter_format(
    format: &ParameterSchemaOrContent,
    refs: &mut Refs,
    worklist: &mut Vec<WorkItem>,
) {
    match format {
        ParameterSchemaOrContent::Schema(schema) => scan_schema_ref(schema, refs, worklist),
        ParameterSchemaOrContent::Content(content) => scan_content(content, refs, worklist),
    }
}

fn scan_request_body_ref(
    request_body: &ReferenceOr<RequestBody>,
    refs: &mut Refs,
    worklist: &mut Vec<WorkItem>,
) {
    match request_body {
        ReferenceOr::Reference { reference } => add_component_ref(reference, refs, worklist),
        ReferenceOr::Item(request_body) => scan_request_body(request_body, refs, worklist),
    }
}

fn scan_request_body(request_body: &RequestBody, refs: &mut Refs, worklist: &mut Vec<WorkItem>) {
    scan_content(&request_body.content, refs, worklist);
}

fn scan_response_ref(
    response: &ReferenceOr<Response>,
    refs: &mut Refs,
    worklist: &mut Vec<WorkItem>,
) {
    match response {
        ReferenceOr::Reference { reference } => add_component_ref(reference, refs, worklist),
        ReferenceOr::Item(response) => scan_response(response, refs, worklist),
    }
}

fn scan_response(response: &Response, refs: &mut Refs, worklist: &mut Vec<WorkItem>) {
    for header in response.headers.values() {
        scan_header_ref(header, refs, worklist);
    }
    scan_content(&response.content, refs, worklist);
}

fn scan_header_ref(header: &ReferenceOr<Header>, refs: &mut Refs, worklist: &mut Vec<WorkItem>) {
    match header {
        ReferenceOr::Reference { reference } => add_component_ref(reference, refs, worklist),
        ReferenceOr::Item(header) => scan_header(header, refs, worklist),
    }
}

fn scan_header(header: &Header, refs: &mut Refs, worklist: &mut Vec<WorkItem>) {
    scan_parameter_format(&header.format, refs, worklist);
}

fn scan_content(
    content: &indexmap::IndexMap<String, MediaType>,
    refs: &mut Refs,
    worklist: &mut Vec<WorkItem>,
) {
    for media_type in content.values() {
        scan_media_type(media_type, refs, worklist);
    }
}

fn scan_media_type(media_type: &MediaType, refs: &mut Refs, worklist: &mut Vec<WorkItem>) {
    if let Some(schema) = &media_type.schema {
        scan_schema_ref(schema, refs, worklist);
    }
    for encoding in media_type.encoding.values() {
        for header in encoding.headers.values() {
            scan_header_ref(header, refs, worklist);
        }
    }
}

fn scan_schema_ref(schema: &ReferenceOr<Schema>, refs: &mut Refs, worklist: &mut Vec<WorkItem>) {
    match schema {
        ReferenceOr::Reference { reference } => add_component_ref(reference, refs, worklist),
        ReferenceOr::Item(schema) => scan_schema(schema, refs, worklist),
    }
}

fn scan_boxed_schema_ref(
    schema: &ReferenceOr<Box<Schema>>,
    refs: &mut Refs,
    worklist: &mut Vec<WorkItem>,
) {
    match schema {
        ReferenceOr::Reference { reference } => add_component_ref(reference, refs, worklist),
        ReferenceOr::Item(schema) => scan_schema(schema, refs, worklist),
    }
}

fn scan_schema(schema: &Schema, refs: &mut Refs, worklist: &mut Vec<WorkItem>) {
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
    refs: &mut Refs,
    worklist: &mut Vec<WorkItem>,
) {
    for schema in schemas {
        scan_schema_ref(schema, refs, worklist);
    }
}

fn scan_type(typ: &Type, refs: &mut Refs, worklist: &mut Vec<WorkItem>) {
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
    refs: &mut Refs,
    worklist: &mut Vec<WorkItem>,
) {
    match additional {
        AdditionalProperties::Any(_) => {}
        AdditionalProperties::Schema(schema) => scan_schema_ref(schema, refs, worklist),
    }
}

fn add_component_ref(reference: &str, refs: &mut Refs, worklist: &mut Vec<WorkItem>) {
    let Some((kind, name)) = parse_component_ref(reference) else {
        return;
    };
    add_named_ref(kind, &name, refs, worklist);
}

fn add_named_ref(kind: ComponentKind, name: &str, refs: &mut Refs, worklist: &mut Vec<WorkItem>) {
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

fn parse_component_ref(reference: &str) -> Option<(ComponentKind, String)> {
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

fn prune_components(api: &mut OpenAPI, refs: &Refs) -> PrunedComponents {
    let Some(components) = api.components.as_mut() else {
        return PrunedComponents::default();
    };

    let mut report = PrunedComponents {
        schemas_before: components.schemas.len(),
        responses_before: components.responses.len(),
        parameters_before: components.parameters.len(),
        request_bodies_before: components.request_bodies.len(),
        headers_before: components.headers.len(),
        security_schemes_before: components.security_schemes.len(),
        ..Default::default()
    };

    components
        .schemas
        .retain(|name, _| refs.schemas.contains(name));
    components
        .responses
        .retain(|name, _| refs.responses.contains(name));
    components
        .parameters
        .retain(|name, _| refs.parameters.contains(name));
    components
        .request_bodies
        .retain(|name, _| refs.request_bodies.contains(name));
    components
        .headers
        .retain(|name, _| refs.headers.contains(name));
    components
        .security_schemes
        .retain(|name, _| refs.security_schemes.contains(name));
    components.examples.clear();
    components.links.clear();
    components.callbacks.clear();

    report.schemas_after = components.schemas.len();
    report.responses_after = components.responses.len();
    report.parameters_after = components.parameters.len();
    report.request_bodies_after = components.request_bodies.len();
    report.headers_after = components.headers.len();
    report.security_schemes_after = components.security_schemes.len();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::report::ReportStage;

    #[test]
    fn filters_and_prunes_components() {
        let mut api: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info: { title: Slice Test, version: '1.0' }
paths:
  /kept/{id}:
    parameters:
      - $ref: '#/components/parameters/Id'
    get:
      operationId: kept/get
      tags: [kept]
      responses:
        '200':
          $ref: '#/components/responses/Kept'
  /dropped:
    get:
      operationId: dropped/get
      tags: [dropped]
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Dropped'
components:
  parameters:
    Id:
      name: id
      in: path
      required: true
      schema: { type: string }
  responses:
    Kept:
      description: ok
      content:
        application/json:
          schema:
            $ref: '#/components/schemas/Kept'
  schemas:
    Kept:
      type: object
      properties:
        shared:
          $ref: '#/components/schemas/Shared'
    Shared:
      type: string
    Dropped:
      type: object
"#,
        )
        .unwrap();

        let report = slice_openapi(
            &mut api,
            &SliceOptions {
                include_tags: vec!["kept".to_string()],
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(report.kept_operations, 1);
        assert_eq!(report.dropped_operations, 1);
        let report_entries = report.report_entries();
        assert_eq!(report_entries.len(), 2);
        assert_eq!(report_entries[0].stage, ReportStage::Slicing);
        assert_eq!(report_entries[0].code, "spec.slice.operations_filtered");
        assert_eq!(
            report_entries[0].formatted_warning(),
            "sliced spec — kept 1 operations, dropped 1 operations"
        );
        assert_eq!(
            report_entries[1].subject,
            Some(ReportSubject::component("components"))
        );
        assert!(api.paths.paths.contains_key("/kept/{id}"));
        assert!(!api.paths.paths.contains_key("/dropped"));
        let components = api.components.as_ref().unwrap();
        assert!(components.schemas.contains_key("Kept"));
        assert!(components.schemas.contains_key("Shared"));
        assert!(!components.schemas.contains_key("Dropped"));
        assert!(components.parameters.contains_key("Id"));
        assert!(components.responses.contains_key("Kept"));
        assert_eq!(list_operations(&api).len(), 1);
    }

    #[test]
    fn prunes_request_bodies_headers_and_security_schemes() {
        let mut api: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info: { title: Slice Component Test, version: '1.0' }
security:
  - KeptKey: []
paths:
  /kept:
    post:
      operationId: keptPost
      requestBody:
        $ref: '#/components/requestBodies/KeptBody'
      responses:
        '200':
          description: ok
          headers:
            X-Kept:
              $ref: '#/components/headers/KeptHeader'
  /dropped:
    post:
      operationId: droppedPost
      security:
        - DroppedKey: []
      requestBody:
        $ref: '#/components/requestBodies/DroppedBody'
      responses:
        '200':
          description: ok
          headers:
            X-Dropped:
              $ref: '#/components/headers/DroppedHeader'
components:
  requestBodies:
    KeptBody:
      content:
        application/json:
          schema:
            $ref: '#/components/schemas/KeptBodySchema'
    DroppedBody:
      content:
        application/json:
          schema:
            $ref: '#/components/schemas/DroppedBodySchema'
  headers:
    KeptHeader:
      schema:
        $ref: '#/components/schemas/KeptHeaderSchema'
    DroppedHeader:
      schema:
        $ref: '#/components/schemas/DroppedHeaderSchema'
  securitySchemes:
    KeptKey:
      type: apiKey
      in: header
      name: X-Kept-Key
    DroppedKey:
      type: apiKey
      in: header
      name: X-Dropped-Key
  schemas:
    KeptBodySchema:
      type: object
    KeptHeaderSchema:
      type: string
    DroppedBodySchema:
      type: object
    DroppedHeaderSchema:
      type: string
"#,
        )
        .unwrap();

        let report = slice_openapi(
            &mut api,
            &SliceOptions {
                include_operations: vec!["keptPost".to_string()],
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(report.kept_operations, 1);
        assert_eq!(report.dropped_operations, 1);
        assert_eq!(report.pruned_components.request_bodies_before, 2);
        assert_eq!(report.pruned_components.request_bodies_after, 1);
        assert_eq!(report.pruned_components.headers_before, 2);
        assert_eq!(report.pruned_components.headers_after, 1);
        assert_eq!(report.pruned_components.security_schemes_before, 2);
        assert_eq!(report.pruned_components.security_schemes_after, 1);

        let components = api.components.as_ref().unwrap();
        assert!(components.request_bodies.contains_key("KeptBody"));
        assert!(!components.request_bodies.contains_key("DroppedBody"));
        assert!(components.headers.contains_key("KeptHeader"));
        assert!(!components.headers.contains_key("DroppedHeader"));
        assert!(components.security_schemes.contains_key("KeptKey"));
        assert!(!components.security_schemes.contains_key("DroppedKey"));
        assert!(components.schemas.contains_key("KeptBodySchema"));
        assert!(components.schemas.contains_key("KeptHeaderSchema"));
        assert!(!components.schemas.contains_key("DroppedBodySchema"));
        assert!(!components.schemas.contains_key("DroppedHeaderSchema"));
    }
}
