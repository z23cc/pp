use crate::spec::{OperationRef, PpParameterRef, PpSpec};
use serde_json::Value;

const METHODS: &[(&str, &str)] = &[
    ("get", "GET"),
    ("put", "PUT"),
    ("post", "POST"),
    ("delete", "DELETE"),
    ("options", "OPTIONS"),
    ("head", "HEAD"),
    ("patch", "PATCH"),
    ("trace", "TRACE"),
];

pub(crate) fn operations(spec: &PpSpec) -> Vec<OperationRef<'_>> {
    let mut out = Vec::new();
    let Some(paths) = spec.document().get("paths").and_then(Value::as_object) else {
        return out;
    };
    for (path, path_item) in paths {
        let Some(item) = path_item.as_object() else {
            continue;
        };
        let path_parameters = item
            .get("parameters")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(PpParameterRef::new)
            .collect::<Vec<_>>();
        for (method, method_uppercase) in METHODS {
            if let Some(operation) = item.get(*method) {
                out.push(OperationRef::new(
                    method,
                    method_uppercase,
                    path,
                    path_parameters.clone(),
                    operation,
                ));
            }
        }
    }
    out
}

#[allow(dead_code)]
pub(crate) fn explicit_operation_id<'a>(operation: &'a OperationRef<'a>) -> Option<&'a str> {
    operation.explicit_operation_id()
}

pub(crate) fn derived_operation_identifier(method: &str, path: &str) -> String {
    format!("{method} {path}")
}

#[allow(dead_code)]
pub(crate) fn operation_identifier(operation: &OperationRef<'_>) -> String {
    explicit_operation_id(operation)
        .map(str::to_string)
        .unwrap_or_else(|| derived_operation_identifier(operation.method, operation.path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_preserve_path_and_method_order() {
        let spec = crate::spec::parse_spec_for_tests(
            r#"
openapi: 3.0.0
info: { title: Traversal Order, version: '1.0' }
paths:
  /first:
    patch:
      operationId: firstPatch
      responses: { '200': { description: ok } }
    get:
      operationId: firstGet
      responses: { '200': { description: ok } }
    post:
      operationId: firstPost
      responses: { '200': { description: ok } }
  /second:
    trace:
      operationId: secondTrace
      responses: { '200': { description: ok } }
    put:
      operationId: secondPut
      responses: { '200': { description: ok } }
"#,
        )
        .unwrap();

        let got = operations(&spec)
            .into_iter()
            .map(|op| (op.method, op.method_uppercase, operation_identifier(&op)))
            .collect::<Vec<_>>();

        assert_eq!(
            got,
            vec![
                ("get", "GET", "firstGet".to_string()),
                ("post", "POST", "firstPost".to_string()),
                ("patch", "PATCH", "firstPatch".to_string()),
                ("put", "PUT", "secondPut".to_string()),
                ("trace", "TRACE", "secondTrace".to_string()),
            ]
        );
    }

    #[test]
    fn operation_ref_exposes_path_level_parameters() {
        let spec = crate::spec::parse_spec_for_tests(
            r#"
openapi: 3.0.0
info: { title: Traversal Params, version: '1.0' }
paths:
  /items/{id}:
    parameters:
      - name: id
        in: path
        required: true
        schema: { type: string }
      - name: api_key
        in: query
        required: true
        schema: { type: string }
    get:
      operationId: getItem
      responses: { '200': { description: ok } }
"#,
        )
        .unwrap();

        let ops = operations(&spec);
        assert_eq!(ops.len(), 1);
        let names = ops[0]
            .parameters()
            .into_iter()
            .map(|param| {
                param
                    .item()
                    .map(|param| param.name().unwrap_or("<unnamed>").to_string())
                    .unwrap_or_else(|| panic!("unexpected parameter"))
            })
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["id".to_string(), "api_key".to_string()]);
    }
}
