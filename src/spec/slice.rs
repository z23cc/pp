use super::preparation_rules::{self as rules, slicing};
use super::references::{collect_reachable_components_value, ComponentRefs};
use super::report::{ReportEntry, ReportSubject};
use super::traversal;
use crate::spec::PpSpec;
use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::{Map, Value};

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
    pub operation_id: Option<String>,
    pub derived_id: String,
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

pub fn list_operations(spec: &PpSpec) -> Vec<OperationListing> {
    traversal::operations(spec)
        .into_iter()
        .map(|operation| {
            let derived_id =
                traversal::derived_operation_identifier(operation.method, operation.path);
            let operation_id = operation.explicit_operation_id().map(str::to_string);
            OperationListing {
                id: operation_id.clone().unwrap_or_else(|| derived_id.clone()),
                method: operation.method.to_string(),
                path: operation.path.to_string(),
                tags: operation.tags(),
                generatable: operation_id.is_some(),
                operation_id,
                derived_id,
            }
        })
        .collect()
}

pub fn slice_spec(spec: &mut PpSpec, options: &SliceOptions) -> Result<SliceReport> {
    if options.is_noop() {
        return Ok(SliceReport::default());
    }

    let mut kept_operations = 0;
    let mut dropped_operations = 0;
    let has_includes = options.has_includes();
    let Some(paths) = spec
        .document_mut()
        .get_mut("paths")
        .and_then(Value::as_object_mut)
    else {
        bail!("OpenAPI document is missing object field 'paths'");
    };

    for (path, path_item) in paths.iter_mut() {
        let Some(item) = path_item.as_object_mut() else {
            if has_includes {
                dropped_operations += 1;
            }
            continue;
        };
        for method in [
            "get", "put", "post", "delete", "options", "head", "patch", "trace",
        ] {
            filter_operation_slot(
                method,
                path,
                item,
                options,
                &mut kept_operations,
                &mut dropped_operations,
            );
        }
    }

    paths.retain(|_, path_item| {
        path_item
            .as_object()
            .map(path_item_has_operations)
            .unwrap_or(!has_includes)
    });

    if kept_operations == 0 {
        bail!(
            "no operations matched slice filters; use `pp inspect --list-operations` to discover operation IDs/tags"
        );
    }

    drop_codegen_ignored_graph_roots(spec.document_mut());
    let refs = collect_reachable_components_value(spec.document());
    let pruned_components = prune_components(spec.document_mut(), &refs);

    Ok(SliceReport {
        kept_operations,
        dropped_operations,
        pruned_components,
    })
}

fn filter_operation_slot(
    method: &str,
    path: &str,
    item: &mut Map<String, Value>,
    options: &SliceOptions,
    kept: &mut usize,
    dropped: &mut usize,
) {
    let Some(op) = item.get(method) else {
        return;
    };
    if operation_matches(method, path, op, options) {
        *kept += 1;
    } else {
        item.remove(method);
        *dropped += 1;
    }
}

fn operation_matches(method: &str, path: &str, operation: &Value, options: &SliceOptions) -> bool {
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
        || options.include_tags.iter().any(|tag| {
            operation
                .get("tags")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|op_tag| op_tag == tag)
        })
        || options
            .include_path_prefixes
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

fn operation_identifier(method: &str, path: &str, operation: &Value) -> String {
    operation
        .get("operationId")
        .and_then(Value::as_str)
        .filter(|operation_id| !operation_id.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| traversal::derived_operation_identifier(method, path))
}

fn path_item_has_operations(item: &Map<String, Value>) -> bool {
    [
        "get", "put", "post", "delete", "options", "head", "patch", "trace",
    ]
    .iter()
    .any(|method| item.contains_key(*method))
}

fn drop_codegen_ignored_graph_roots(doc: &mut Value) {
    if let Some(paths) = doc.get_mut("paths").and_then(Value::as_object_mut) {
        for path_item in paths.values_mut().filter_map(Value::as_object_mut) {
            for method in [
                "get", "put", "post", "delete", "options", "head", "patch", "trace",
            ] {
                if let Some(operation) = path_item.get_mut(method).and_then(Value::as_object_mut) {
                    operation.remove("callbacks");
                    clear_response_links(operation.get_mut("responses"));
                }
            }
        }
    }
    if let Some(components) = doc.get_mut("components").and_then(Value::as_object_mut) {
        components.remove("examples");
        components.remove("links");
        components.remove("callbacks");
        if let Some(responses) = components
            .get_mut("responses")
            .and_then(Value::as_object_mut)
        {
            for response in responses.values_mut() {
                clear_response_links(Some(response));
            }
        }
    }
}

fn clear_response_links(value: Option<&mut Value>) {
    let Some(value) = value else {
        return;
    };
    if let Some(object) = value.as_object_mut() {
        object.remove("links");
        for response in object.values_mut() {
            if let Some(response_object) = response.as_object_mut() {
                response_object.remove("links");
            }
        }
    }
}

fn prune_components(doc: &mut Value, refs: &ComponentRefs) -> PrunedComponents {
    let Some(components) = doc.get_mut("components").and_then(Value::as_object_mut) else {
        return PrunedComponents::default();
    };

    let mut report = PrunedComponents {
        schemas_before: component_len(components, "schemas"),
        responses_before: component_len(components, "responses"),
        parameters_before: component_len(components, "parameters"),
        request_bodies_before: component_len(components, "requestBodies"),
        headers_before: component_len(components, "headers"),
        security_schemes_before: component_len(components, "securitySchemes"),
        ..Default::default()
    };

    retain_component_map(components, "schemas", &refs.schemas);
    retain_component_map(components, "responses", &refs.responses);
    retain_component_map(components, "parameters", &refs.parameters);
    retain_component_map(components, "requestBodies", &refs.request_bodies);
    retain_component_map(components, "headers", &refs.headers);
    retain_component_map(components, "securitySchemes", &refs.security_schemes);
    components.remove("examples");
    components.remove("links");
    components.remove("callbacks");

    report.schemas_after = component_len(components, "schemas");
    report.responses_after = component_len(components, "responses");
    report.parameters_after = component_len(components, "parameters");
    report.request_bodies_after = component_len(components, "requestBodies");
    report.headers_after = component_len(components, "headers");
    report.security_schemes_after = component_len(components, "securitySchemes");
    report
}

fn component_len(components: &Map<String, Value>, name: &str) -> usize {
    components
        .get(name)
        .and_then(Value::as_object)
        .map(Map::len)
        .unwrap_or(0)
}

fn retain_component_map(
    components: &mut Map<String, Value>,
    name: &str,
    keep: &std::collections::BTreeSet<String>,
) {
    if let Some(map) = components.get_mut(name).and_then(Value::as_object_mut) {
        map.retain(|component_name, _| keep.contains(component_name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::report::ReportStage;

    #[test]
    fn list_operations_uses_method_path_derived_id_without_operation_id() {
        let api = crate::spec::parse_spec_for_tests(
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
        let mut api = crate::spec::parse_spec_for_tests(
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

        let report = slice_spec(
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
        assert!(api.document().pointer("/paths/~1kept~1{id}").is_some());
        assert!(api.document().pointer("/paths/~1dropped").is_none());
        let components = api
            .document()
            .get("components")
            .and_then(Value::as_object)
            .unwrap();
        assert!(components["schemas"]
            .as_object()
            .unwrap()
            .contains_key("Kept"));
        assert!(components["schemas"]
            .as_object()
            .unwrap()
            .contains_key("Shared"));
        assert!(!components["schemas"]
            .as_object()
            .unwrap()
            .contains_key("Dropped"));
        assert!(components["parameters"]
            .as_object()
            .unwrap()
            .contains_key("Id"));
        assert!(components["responses"]
            .as_object()
            .unwrap()
            .contains_key("Kept"));
        assert_eq!(list_operations(&api).len(), 1);
    }
}
