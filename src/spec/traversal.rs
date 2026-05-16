use openapiv3::{OpenAPI, Operation, Parameter, ReferenceOr};

#[derive(Debug, Clone, Copy)]
pub(crate) struct OperationRef<'a> {
    pub method: &'static str,
    pub method_uppercase: &'static str,
    pub path: &'a str,
    pub path_parameters: &'a [ReferenceOr<Parameter>],
    pub operation: &'a Operation,
}

pub(crate) struct OperationSlotMut<'a> {
    pub method: &'static str,
    pub method_uppercase: &'static str,
    pub path: &'a str,
    pub path_parameters: &'a [ReferenceOr<Parameter>],
    pub operation: &'a mut Option<Operation>,
}

#[allow(dead_code)]
pub(crate) struct OperationRefMut<'a> {
    pub method: &'static str,
    pub method_uppercase: &'static str,
    pub path: &'a str,
    pub path_parameters: &'a [ReferenceOr<Parameter>],
    pub operation: &'a mut Operation,
}

pub(crate) fn operations(api: &OpenAPI) -> Vec<OperationRef<'_>> {
    let mut out = Vec::new();
    for (path, path_item) in api.paths.iter() {
        let ReferenceOr::Item(item) = path_item else {
            continue;
        };
        let path_parameters = item.parameters.as_slice();
        if let Some(operation) = item.get.as_ref() {
            out.push(operation_ref(
                "get",
                "GET",
                path,
                path_parameters,
                operation,
            ));
        }
        if let Some(operation) = item.put.as_ref() {
            out.push(operation_ref(
                "put",
                "PUT",
                path,
                path_parameters,
                operation,
            ));
        }
        if let Some(operation) = item.post.as_ref() {
            out.push(operation_ref(
                "post",
                "POST",
                path,
                path_parameters,
                operation,
            ));
        }
        if let Some(operation) = item.delete.as_ref() {
            out.push(operation_ref(
                "delete",
                "DELETE",
                path,
                path_parameters,
                operation,
            ));
        }
        if let Some(operation) = item.options.as_ref() {
            out.push(operation_ref(
                "options",
                "OPTIONS",
                path,
                path_parameters,
                operation,
            ));
        }
        if let Some(operation) = item.head.as_ref() {
            out.push(operation_ref(
                "head",
                "HEAD",
                path,
                path_parameters,
                operation,
            ));
        }
        if let Some(operation) = item.patch.as_ref() {
            out.push(operation_ref(
                "patch",
                "PATCH",
                path,
                path_parameters,
                operation,
            ));
        }
        if let Some(operation) = item.trace.as_ref() {
            out.push(operation_ref(
                "trace",
                "TRACE",
                path,
                path_parameters,
                operation,
            ));
        }
    }
    out
}

pub(crate) fn visit_operation_slots_mut(
    api: &mut OpenAPI,
    mut visitor: impl FnMut(OperationSlotMut<'_>),
) {
    for (path, path_item) in api.paths.paths.iter_mut() {
        let ReferenceOr::Item(item) = path_item else {
            continue;
        };
        let path_parameters = item.parameters.as_slice();
        visitor(operation_slot_mut(
            "get",
            "GET",
            path,
            path_parameters,
            &mut item.get,
        ));
        visitor(operation_slot_mut(
            "put",
            "PUT",
            path,
            path_parameters,
            &mut item.put,
        ));
        visitor(operation_slot_mut(
            "post",
            "POST",
            path,
            path_parameters,
            &mut item.post,
        ));
        visitor(operation_slot_mut(
            "delete",
            "DELETE",
            path,
            path_parameters,
            &mut item.delete,
        ));
        visitor(operation_slot_mut(
            "options",
            "OPTIONS",
            path,
            path_parameters,
            &mut item.options,
        ));
        visitor(operation_slot_mut(
            "head",
            "HEAD",
            path,
            path_parameters,
            &mut item.head,
        ));
        visitor(operation_slot_mut(
            "patch",
            "PATCH",
            path,
            path_parameters,
            &mut item.patch,
        ));
        visitor(operation_slot_mut(
            "trace",
            "TRACE",
            path,
            path_parameters,
            &mut item.trace,
        ));
    }
}

pub(crate) fn visit_operations_mut(
    api: &mut OpenAPI,
    mut visitor: impl FnMut(OperationRefMut<'_>),
) {
    visit_operation_slots_mut(api, |slot| {
        let OperationSlotMut {
            method,
            method_uppercase,
            path,
            path_parameters,
            operation,
        } = slot;
        if let Some(operation) = operation.as_mut() {
            visitor(OperationRefMut {
                method,
                method_uppercase,
                path,
                path_parameters,
                operation,
            });
        }
    });
}

pub(crate) fn operation_identifier(method: &str, path: &str, operation: &Operation) -> String {
    operation
        .operation_id
        .clone()
        .unwrap_or_else(|| format!("{method} {path}"))
}

fn operation_ref<'a>(
    method: &'static str,
    method_uppercase: &'static str,
    path: &'a str,
    path_parameters: &'a [ReferenceOr<Parameter>],
    operation: &'a Operation,
) -> OperationRef<'a> {
    OperationRef {
        method,
        method_uppercase,
        path,
        path_parameters,
        operation,
    }
}

fn operation_slot_mut<'a>(
    method: &'static str,
    method_uppercase: &'static str,
    path: &'a str,
    path_parameters: &'a [ReferenceOr<Parameter>],
    operation: &'a mut Option<Operation>,
) -> OperationSlotMut<'a> {
    OperationSlotMut {
        method,
        method_uppercase,
        path,
        path_parameters,
        operation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openapiv3::Parameter;

    #[test]
    fn operations_preserve_path_and_method_order() {
        let api: OpenAPI = serde_yaml::from_str(
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

        let got = operations(&api)
            .into_iter()
            .map(|op| {
                (
                    op.method,
                    op.method_uppercase,
                    operation_identifier(op.method, op.path, op.operation),
                )
            })
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
        let api: OpenAPI = serde_yaml::from_str(
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

        let ops = operations(&api);
        assert_eq!(ops.len(), 1);
        let names = ops[0]
            .path_parameters
            .iter()
            .map(|param| match param {
                ReferenceOr::Item(Parameter::Path { parameter_data, .. })
                | ReferenceOr::Item(Parameter::Query { parameter_data, .. }) => {
                    parameter_data.name.as_str()
                }
                _ => panic!("unexpected parameter"),
            })
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["id", "api_key"]);
    }

    #[test]
    fn mutable_operations_preserve_path_and_method_order() {
        let mut api: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info: { title: Mutable Traversal Order, version: '1.0' }
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

        let mut visited = Vec::new();
        visit_operations_mut(&mut api, |operation_ref| {
            visited.push((
                operation_ref.method,
                operation_ref.method_uppercase,
                operation_ref.path.to_string(),
                operation_ref.operation.operation_id.clone().unwrap(),
            ));
            operation_ref.operation.deprecated = true;
        });

        assert_eq!(
            visited,
            vec![
                ("get", "GET", "/first".to_string(), "firstGet".to_string()),
                (
                    "post",
                    "POST",
                    "/first".to_string(),
                    "firstPost".to_string()
                ),
                (
                    "patch",
                    "PATCH",
                    "/first".to_string(),
                    "firstPatch".to_string()
                ),
                ("put", "PUT", "/second".to_string(), "secondPut".to_string()),
                (
                    "trace",
                    "TRACE",
                    "/second".to_string(),
                    "secondTrace".to_string()
                ),
            ]
        );
        assert!(operations(&api)
            .into_iter()
            .all(|op| op.operation.deprecated));
    }

    #[test]
    fn mutable_operation_slots_can_drop_operations() {
        let mut api: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info: { title: Mutable Traversal Drop, version: '1.0' }
paths:
  /items:
    get:
      operationId: listItems
      responses: { '200': { description: ok } }
    post:
      operationId: createItem
      responses: { '200': { description: ok } }
"#,
        )
        .unwrap();

        visit_operation_slots_mut(&mut api, |slot| {
            if slot.method == "post" {
                *slot.operation = None;
            }
        });

        let got = operations(&api)
            .into_iter()
            .map(|op| operation_identifier(op.method, op.path, op.operation))
            .collect::<Vec<_>>();
        assert_eq!(got, vec!["listItems".to_string()]);
    }
}
