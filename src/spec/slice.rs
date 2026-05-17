use super::preparation_rules::{self as rules, slicing};
use super::references::{collect_reachable_components, ComponentRefs};
use super::report::{ReportEntry, ReportSubject};
use super::traversal;
use anyhow::{bail, Result};
use openapiv3::{OpenAPI, Operation, PathItem, ReferenceOr, Response};
use serde::Serialize;

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
    /// Discovery identifier: explicit operationId when present, otherwise a
    /// method/path label used only for inspection and slicing output.
    pub id: String,
    pub method: String,
    pub path: String,
    pub tags: Vec<String>,
    /// The explicit OpenAPI operationId used by codegen/MCP identity.
    pub operation_id: Option<String>,
    /// A discovery-only identifier derived from method and path.
    pub derived_id: String,
    /// Whether this operation can be used by generation/model paths without
    /// first adding an explicit operationId to the source spec.
    pub generatable: bool,
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
    traversal::operations(api)
        .into_iter()
        .map(|operation| {
            let derived_id =
                traversal::derived_operation_identifier(operation.method, operation.path);
            let operation_id =
                traversal::explicit_operation_id(operation.operation).map(str::to_string);
            OperationListing {
                id: operation_id.clone().unwrap_or_else(|| derived_id.clone()),
                method: operation.method.to_string(),
                path: operation.path.to_string(),
                tags: operation.operation.tags.clone(),
                generatable: operation_id.is_some(),
                operation_id,
                derived_id,
            }
        })
        .collect()
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
    let id = traversal::operation_identifier(method, path, operation);
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

fn prune_components(api: &mut OpenAPI, refs: &ComponentRefs) -> PrunedComponents {
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
    fn list_operations_uses_method_path_derived_id_without_operation_id() {
        let api: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info: { title: Derived Operation IDs, version: '1.0' }
paths:
  /items/{id}:
    patch:
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();

        let operations = list_operations(&api);

        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].id, "patch /items/{id}");
        assert_eq!(operations[0].method, "patch");
        assert_eq!(operations[0].path, "/items/{id}");
        assert_eq!(operations[0].operation_id, None);
        assert_eq!(operations[0].derived_id, "patch /items/{id}");
        assert!(!operations[0].generatable);
    }

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
