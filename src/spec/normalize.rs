use openapiv3::{MediaType, OpenAPI, Operation, ReferenceOr, RequestBody, Response, StatusCode};

const JSON_MIME: &str = "application/json";

pub fn normalize(spec: &mut OpenAPI) -> Vec<String> {
    let mut warnings = Vec::new();

    if let Some(components) = spec.components.as_mut() {
        for (name, request_body) in components.request_bodies.iter_mut() {
            if let ReferenceOr::Item(request_body) = request_body {
                normalize_request_body(
                    request_body,
                    &format!("component requestBody {name}"),
                    &mut warnings,
                );
            }
        }
        for (name, response) in components.responses.iter_mut() {
            if let ReferenceOr::Item(response) = response {
                normalize_response(
                    response,
                    &format!("component response {name}"),
                    &mut warnings,
                );
            }
        }
    }

    for (path, path_item) in spec.paths.paths.iter_mut() {
        let ReferenceOr::Item(item) = path_item else {
            continue;
        };

        normalize_maybe_operation("get", path, &mut item.get, &mut warnings);
        normalize_maybe_operation("put", path, &mut item.put, &mut warnings);
        normalize_maybe_operation("post", path, &mut item.post, &mut warnings);
        normalize_maybe_operation("delete", path, &mut item.delete, &mut warnings);
        normalize_maybe_operation("options", path, &mut item.options, &mut warnings);
        normalize_maybe_operation("head", path, &mut item.head, &mut warnings);
        normalize_maybe_operation("patch", path, &mut item.patch, &mut warnings);
        normalize_maybe_operation("trace", path, &mut item.trace, &mut warnings);
    }

    warnings
}

fn normalize_maybe_operation(
    method: &str,
    path: &str,
    operation: &mut Option<Operation>,
    warnings: &mut Vec<String>,
) {
    let Some(operation) = operation else {
        return;
    };
    let op_name = operation_name(method, path, operation);
    normalize_operation(operation, &op_name, warnings);
}

fn normalize_operation(operation: &mut Operation, op_name: &str, warnings: &mut Vec<String>) {
    normalize_response_variants(operation, op_name, warnings);

    if let Some(ReferenceOr::Item(request_body)) = operation.request_body.as_mut() {
        normalize_request_body(request_body, op_name, warnings);
    }

    for response in operation.responses.responses.values_mut() {
        if let ReferenceOr::Item(response) = response {
            normalize_response(response, op_name, warnings);
        }
    }
    if let Some(ReferenceOr::Item(response)) = operation.responses.default.as_mut() {
        normalize_response(response, op_name, warnings);
    }
}

fn normalize_response_variants(
    operation: &mut Operation,
    op_name: &str,
    warnings: &mut Vec<String>,
) {
    let mut codes: Vec<String> = operation
        .responses
        .responses
        .keys()
        .map(ToString::to_string)
        .collect();
    if operation.responses.default.is_some() {
        codes.push("default".to_string());
    }
    if codes.len() <= 1 {
        return;
    }

    codes.sort();
    let kept = if operation
        .responses
        .responses
        .contains_key(&StatusCode::Code(200))
    {
        "200".to_string()
    } else if let Some(code) = codes
        .iter()
        .find(|code| code.starts_with('2') && code.as_str() != "200")
    {
        code.clone()
    } else {
        codes[0].clone()
    };
    let dropped: Vec<String> = codes.into_iter().filter(|code| code != &kept).collect();

    operation
        .responses
        .responses
        .retain(|code, _| code.to_string() == kept);
    if kept != "default" {
        operation.responses.default = None;
    }

    warnings.push(format!(
        "normalized {op_name} responses — kept {kept}, dropped {}",
        dropped.join(", ")
    ));
}

fn normalize_request_body(
    request_body: &mut RequestBody,
    op_name: &str,
    warnings: &mut Vec<String>,
) {
    if let Some((kept, dropped)) = normalize_content(&mut request_body.content) {
        warnings.push(format!(
            "normalized {op_name} — kept {kept}, dropped {}",
            dropped.join(", ")
        ));
    }
}

fn normalize_response(response: &mut Response, op_name: &str, warnings: &mut Vec<String>) {
    if let Some((kept, dropped)) = normalize_content(&mut response.content) {
        warnings.push(format!(
            "normalized {op_name} — kept {kept}, dropped {}",
            dropped.join(", ")
        ));
    }
}

fn normalize_content(
    content: &mut indexmap::IndexMap<String, MediaType>,
) -> Option<(String, Vec<String>)> {
    if content.len() <= 1 {
        return None;
    }

    let kept = if content.contains_key(JSON_MIME) {
        JSON_MIME.to_string()
    } else {
        content.keys().min().expect("content has entries").clone()
    };
    let dropped: Vec<String> = content
        .keys()
        .filter(|mime| *mime != &kept)
        .cloned()
        .collect();
    let media_type = content.get(&kept).expect("kept media type exists").clone();
    content.clear();
    content.insert(kept.clone(), media_type);

    Some((kept, dropped))
}

fn operation_name(method: &str, path: &str, operation: &Operation) -> String {
    operation
        .operation_id
        .clone()
        .unwrap_or_else(|| format!("{} {}", method.to_uppercase(), path))
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let warnings = normalize(&mut spec);
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

        let warnings = normalize(&mut spec);
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
}
