use crate::spec::PpSpec;
use serde_json::Value;
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

#[allow(dead_code)]
pub(crate) fn collect_reachable_components(spec: &PpSpec) -> ComponentRefs {
    collect_reachable_components_value(spec.document())
}

pub(crate) fn collect_reachable_components_value(doc: &Value) -> ComponentRefs {
    let mut refs = ComponentRefs::default();
    let mut worklist = Vec::new();
    let mut inherits_root_security = false;

    if let Some(paths) = doc.get("paths").and_then(Value::as_object) {
        for path_item in paths.values() {
            let mut local_refs_seen = BTreeSet::new();
            scan_value(
                path_item.get("parameters"),
                doc,
                doc,
                &mut local_refs_seen,
                &mut refs,
                &mut worklist,
            );
            if let Some(item) = path_item.as_object() {
                for method in [
                    "get", "put", "post", "delete", "options", "head", "patch", "trace",
                ] {
                    if let Some(operation) = item.get(method) {
                        scan_value(
                            Some(operation),
                            doc,
                            doc,
                            &mut local_refs_seen,
                            &mut refs,
                            &mut worklist,
                        );
                        if operation.get("security").is_none() {
                            inherits_root_security = true;
                        }
                    }
                }
            }
        }
    }

    if inherits_root_security {
        scan_security_requirements(doc.get("security"), &mut refs, &mut worklist);
    }

    while let Some(item) = worklist.pop() {
        let name = refs.names[item.name_index].clone();
        let pointer = match item.kind {
            ComponentKind::Schema => {
                format!("/components/schemas/{}", encode_json_pointer_segment(&name))
            }
            ComponentKind::Response => format!(
                "/components/responses/{}",
                encode_json_pointer_segment(&name)
            ),
            ComponentKind::Parameter => format!(
                "/components/parameters/{}",
                encode_json_pointer_segment(&name)
            ),
            ComponentKind::RequestBody => format!(
                "/components/requestBodies/{}",
                encode_json_pointer_segment(&name)
            ),
            ComponentKind::Header => {
                format!("/components/headers/{}", encode_json_pointer_segment(&name))
            }
            ComponentKind::SecurityScheme => continue,
        };
        if let Some(component) = doc.pointer(&pointer) {
            let mut local_refs_seen = BTreeSet::new();
            scan_value(
                Some(component),
                doc,
                component,
                &mut local_refs_seen,
                &mut refs,
                &mut worklist,
            );
        }
    }

    refs
}

fn scan_value(
    value: Option<&Value>,
    doc: &Value,
    scope_root: &Value,
    local_refs_seen: &mut BTreeSet<String>,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    match value {
        Some(current @ Value::Object(object)) => {
            let child_scope_root = if object.contains_key("$defs") {
                current
            } else {
                scope_root
            };
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                if !add_component_ref(reference, refs, worklist) {
                    scan_local_ref(
                        reference,
                        doc,
                        child_scope_root,
                        local_refs_seen,
                        refs,
                        worklist,
                    );
                }
            }
            if let Some(requirements) = object.get("security") {
                scan_security_requirements(Some(requirements), refs, worklist);
            }
            for value in object.values() {
                scan_value(
                    Some(value),
                    doc,
                    child_scope_root,
                    local_refs_seen,
                    refs,
                    worklist,
                );
            }
        }
        Some(Value::Array(values)) => {
            for value in values {
                scan_value(
                    Some(value),
                    doc,
                    scope_root,
                    local_refs_seen,
                    refs,
                    worklist,
                );
            }
        }
        _ => {}
    }
}

fn scan_local_ref(
    reference: &str,
    doc: &Value,
    scope_root: &Value,
    local_refs_seen: &mut BTreeSet<String>,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    let Some(local_pointer) = reference.strip_prefix('#') else {
        return;
    };
    let key = format!("{:p}:{reference}", scope_root);
    if !local_refs_seen.insert(key.clone()) {
        return;
    }
    if let Some(target) = scope_root
        .pointer(local_pointer)
        .or_else(|| doc.pointer(local_pointer))
    {
        scan_value(
            Some(target),
            doc,
            scope_root,
            local_refs_seen,
            refs,
            worklist,
        );
    }
    local_refs_seen.remove(&key);
}

fn scan_security_requirements(
    value: Option<&Value>,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) {
    let Some(requirements) = value.and_then(Value::as_array) else {
        return;
    };
    for requirement in requirements.iter().filter_map(Value::as_object) {
        for name in requirement.keys() {
            add_named_ref(ComponentKind::SecurityScheme, name, refs, worklist);
        }
    }
}

fn add_component_ref(
    reference: &str,
    refs: &mut ComponentRefs,
    worklist: &mut Vec<WorkItem>,
) -> bool {
    let Some((kind, name)) = parse_component_ref(reference) else {
        return false;
    };
    add_named_ref(kind, &name, refs, worklist);
    true
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

fn encode_json_pointer_segment(input: &str) -> String {
    input.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn reachable_components_follow_schema_local_defs_aliases() {
        let doc: Value = serde_yaml::from_str(
            r##"
openapi: 3.1.0
info: { title: Local Defs References, version: '1.0' }
paths:
  /items:
    post:
      operationId: createItem
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/Envelope'
      responses:
        '200': { description: ok }
components:
  schemas:
    Envelope:
      type: object
      properties:
        item:
          $ref: '#/$defs/Alias'
      $defs:
        Alias:
          $ref: '#/$defs/TargetAlias'
        TargetAlias:
          $ref: '#/components/schemas/Target'
        CycleA:
          $ref: '#/$defs/CycleB'
        CycleB:
          $ref: '#/$defs/CycleA'
    Target:
      type: object
    Dropped:
      type: object
"##,
        )
        .unwrap();

        let refs = collect_reachable_components_value(&doc);
        assert!(refs.schemas.contains("Envelope"));
        assert!(refs.schemas.contains("Target"));
        assert!(!refs.schemas.contains("Dropped"));
    }

    #[test]
    fn reachable_components_include_nested_schema_and_media_headers() {
        let doc: Value = serde_yaml::from_str(
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
      type: object
      properties:
        child:
          $ref: '#/components/schemas/Child'
    Child:
      type: object
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

        let refs = collect_reachable_components_value(&doc);
        for name in [
            "Envelope",
            "Item",
            "Child",
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
}
