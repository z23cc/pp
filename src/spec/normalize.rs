use anyhow::Result;
use heck::ToSnakeCase;
use openapiv3::{
    ArrayType, MediaType, ObjectType, OpenAPI, Operation, QueryStyle, ReferenceOr, RequestBody,
    Response, Schema, SchemaKind, StatusCode, Type,
};
use std::collections::HashMap;

const VERBOSE_OPERATION_PREFIXES: &[&str] = &[
    "plausible_web_plugins_api_controllers_",
    "PlausibleWeb.Plugins.API.Controllers.",
    "application_controllers_",
];

const JSON_MIME: &str = "application/json";
const FORM_MIME: &str = "application/x-www-form-urlencoded";
const OCTET_STREAM_MIME: &str = "application/octet-stream";

pub fn normalize(spec: &mut OpenAPI) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let mut stats = NormalizeStats::default();
    shorten_verbose_operation_ids(spec);

    if let Some(components) = spec.components.as_mut() {
        for (name, schema) in components.schemas.iter_mut() {
            if let ReferenceOr::Item(schema) = schema {
                normalize_schema(
                    schema,
                    &format!("component schema {name}"),
                    &mut warnings,
                    &mut stats,
                )?;
            }
        }
        for (name, request_body) in components.request_bodies.iter_mut() {
            if let ReferenceOr::Item(request_body) = request_body {
                normalize_request_body(
                    request_body,
                    &format!("component requestBody {name}"),
                    &mut warnings,
                    &mut stats,
                );
            }
        }
        for (name, response) in components.responses.iter_mut() {
            if let ReferenceOr::Item(response) = response {
                normalize_response(
                    response,
                    &format!("component response {name}"),
                    &mut warnings,
                    &mut stats,
                );
            }
        }
    }

    for (path, path_item) in spec.paths.paths.iter_mut() {
        let ReferenceOr::Item(item) = path_item else {
            continue;
        };

        normalize_maybe_operation("get", path, &mut item.get, &mut warnings, &mut stats);
        normalize_maybe_operation("put", path, &mut item.put, &mut warnings, &mut stats);
        normalize_maybe_operation("post", path, &mut item.post, &mut warnings, &mut stats);
        normalize_maybe_operation("delete", path, &mut item.delete, &mut warnings, &mut stats);
        normalize_maybe_operation(
            "options",
            path,
            &mut item.options,
            &mut warnings,
            &mut stats,
        );
        normalize_maybe_operation("head", path, &mut item.head, &mut warnings, &mut stats);
        normalize_maybe_operation("patch", path, &mut item.patch, &mut warnings, &mut stats);
        normalize_maybe_operation("trace", path, &mut item.trace, &mut warnings, &mut stats);
    }

    if stats.dropped_defaults > 0 {
        warnings.push(format!(
            "normalized {} schemas — dropped default values",
            stats.dropped_defaults
        ));
    }
    if !stats.dropped_unsupported_request_body_ops.is_empty() {
        warnings.push(format!(
            "dropped {} operations with progenitor-unsupported request body: {}",
            stats.dropped_unsupported_request_body_ops.len(),
            stats.dropped_unsupported_request_body_ops.join(", ")
        ));
    }
    if !stats.normalized_deep_object_query_params.is_empty() {
        warnings.push(format!(
            "normalized {} query parameters — replaced unsupported deepObject style with form: {}",
            stats.normalized_deep_object_query_params.len(),
            stats.normalized_deep_object_query_params.join(", ")
        ));
    }

    Ok(warnings)
}

#[derive(Default)]
struct NormalizeStats {
    dropped_defaults: usize,
    dropped_unsupported_request_body_ops: Vec<String>,
    normalized_deep_object_query_params: Vec<String>,
}

fn shorten_verbose_operation_ids(spec: &mut OpenAPI) {
    let ids = operation_ids(spec);
    let candidates: Vec<_> = ids
        .iter()
        .filter_map(|old| {
            shorten_candidate(old).map(|new| (old.clone(), new, last_segments(old, 2)))
        })
        .collect();
    let last_three_counts = count_by(candidates.iter().map(|(_, new, _)| new.clone()));
    let chosen: Vec<_> = candidates
        .into_iter()
        .map(|(old, last_three, last_two)| {
            let new = match last_three_counts.get(&last_three) {
                Some(1) => last_three,
                _ => last_two,
            };
            (old, new)
        })
        .collect();
    let chosen_counts = count_by(chosen.iter().map(|(_, new)| new.clone()));
    let replacements: HashMap<_, _> = chosen
        .into_iter()
        .filter(|(old, new)| old != new && chosen_counts.get(new) == Some(&1))
        .collect();

    for operation in operations_mut(spec) {
        if let Some(old) = operation.operation_id.clone() {
            if let Some(new) = replacements.get(&old) {
                operation.operation_id = Some(new.clone());
                eprintln!("pp: shortened operation '{old}' → '{new}'");
            }
        }
    }
}

fn operation_ids(spec: &OpenAPI) -> Vec<String> {
    spec.paths
        .iter()
        .filter_map(|(_, path_item)| match path_item {
            ReferenceOr::Item(item) => Some(item),
            ReferenceOr::Reference { .. } => None,
        })
        .flat_map(|item| {
            [
                &item.get,
                &item.put,
                &item.post,
                &item.delete,
                &item.options,
                &item.head,
                &item.patch,
                &item.trace,
            ]
        })
        .flatten()
        .filter_map(|op| op.operation_id.clone())
        .collect()
}

fn operations_mut(spec: &mut OpenAPI) -> Vec<&mut Operation> {
    spec.paths
        .paths
        .iter_mut()
        .filter_map(|(_, path_item)| match path_item {
            ReferenceOr::Item(item) => Some(item),
            ReferenceOr::Reference { .. } => None,
        })
        .flat_map(|item| {
            [
                &mut item.get,
                &mut item.put,
                &mut item.post,
                &mut item.delete,
                &mut item.options,
                &mut item.head,
                &mut item.patch,
                &mut item.trace,
            ]
        })
        .flatten()
        .collect()
}

fn shorten_candidate(operation_id: &str) -> Option<String> {
    VERBOSE_OPERATION_PREFIXES
        .iter()
        .find_map(|prefix| {
            operation_id
                .strip_prefix(prefix)
                .map(|stripped| stripped.to_snake_case())
        })
        .or_else(|| {
            (operation_segments(operation_id).len() > 4).then(|| last_segments(operation_id, 3))
        })
}
fn last_segments(operation_id: &str, count: usize) -> String {
    let segments = operation_segments(operation_id);
    segments[segments.len().saturating_sub(count)..]
        .join("_")
        .to_snake_case()
}

fn operation_segments(operation_id: &str) -> Vec<&str> {
    operation_id.split(['_', '.']).collect()
}

fn count_by(values: impl Iterator<Item = String>) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    values.for_each(|value| *counts.entry(value).or_insert(0) += 1);
    counts
}

fn normalize_maybe_operation(
    method: &str,
    path: &str,
    operation: &mut Option<Operation>,
    warnings: &mut Vec<String>,
    stats: &mut NormalizeStats,
) {
    let Some(operation_ref) = operation.as_mut() else {
        return;
    };
    let op_name = operation_name(method, path, operation_ref);
    if normalize_operation(operation_ref, &op_name, warnings, stats) {
        stats.dropped_unsupported_request_body_ops.push(op_name);
        *operation = None;
    }
}

fn normalize_operation(
    operation: &mut Operation,
    op_name: &str,
    warnings: &mut Vec<String>,
    stats: &mut NormalizeStats,
) -> bool {
    normalize_response_variants(operation, op_name, warnings);

    for (i, param) in operation.parameters.iter_mut().enumerate() {
        if let ReferenceOr::Item(param) = param {
            let param_data = match param {
                openapiv3::Parameter::Query {
                    parameter_data,
                    style,
                    ..
                } => {
                    if *style == QueryStyle::DeepObject {
                        *style = QueryStyle::Form;
                        stats
                            .normalized_deep_object_query_params
                            .push(format!("{op_name}.{}", parameter_data.name));
                    }
                    parameter_data
                }
                openapiv3::Parameter::Header { parameter_data, .. }
                | openapiv3::Parameter::Path { parameter_data, .. }
                | openapiv3::Parameter::Cookie { parameter_data, .. } => parameter_data,
            };
            if let openapiv3::ParameterSchemaOrContent::Schema(ReferenceOr::Item(schema)) =
                &mut param_data.format
            {
                let _ = normalize_schema(
                    schema,
                    &format!("{op_name}.parameters[{i}].{}", param_data.name),
                    warnings,
                    stats,
                );
            }
        }
    }

    if let Some(ReferenceOr::Item(request_body)) = operation.request_body.as_mut() {
        if normalize_request_body(request_body, op_name, warnings, stats) {
            return true;
        }
        if request_body_has_schemaless_content(request_body) {
            operation.request_body = None;
            warnings.push(format!(
                "normalized {op_name} — dropped requestBody (no schema specified)"
            ));
        }
    }

    for response in operation.responses.responses.values_mut() {
        if let ReferenceOr::Item(response) = response {
            normalize_response(response, op_name, warnings, stats);
        }
    }
    if let Some(ReferenceOr::Item(response)) = operation.responses.default.as_mut() {
        normalize_response(response, op_name, warnings, stats);
    }
    false
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
    stats: &mut NormalizeStats,
) -> bool {
    if let Some((kept, dropped)) = normalize_request_content(&mut request_body.content) {
        warnings.push(format!(
            "normalized {op_name} — kept {kept}, dropped {}",
            dropped.join(", ")
        ));
    }
    if request_body.content.is_empty() {
        return true;
    }
    for (mime, media_type) in request_body.content.iter_mut() {
        if let Some(ReferenceOr::Item(schema)) = media_type.schema.as_mut() {
            let _ = normalize_schema(
                schema,
                &format!("{op_name}.requestBody.{mime}"),
                warnings,
                stats,
            );
        }
    }
    false
}

fn normalize_response(
    response: &mut Response,
    op_name: &str,
    warnings: &mut Vec<String>,
    stats: &mut NormalizeStats,
) {
    if let Some((kept, dropped)) = normalize_content(&mut response.content) {
        warnings.push(format!(
            "normalized {op_name} — kept {kept}, dropped {}",
            dropped.join(", ")
        ));
    }
    for (mime, media_type) in response.content.iter_mut() {
        if let Some(ReferenceOr::Item(schema)) = media_type.schema.as_mut() {
            let _ = normalize_schema(
                schema,
                &format!("{op_name}.response.{mime}"),
                warnings,
                stats,
            );
        }
    }
}

fn normalize_schema(
    schema: &mut Schema,
    path: &str,
    warnings: &mut Vec<String>,
    stats: &mut NormalizeStats,
) -> Result<()> {
    if schema.schema_data.default.take().is_some() {
        stats.dropped_defaults += 1;
    }

    match &mut schema.schema_kind {
        SchemaKind::Type(Type::String(string)) => {
            if let Some(colliding) = string_enum_collision(&string.enumeration) {
                warnings.push(format!(
                    "normalized {path} — dropped enum constraint (values [{colliding}] collide on Rust identifier sanitization); field is now a free-form string preserving wire format"
                ));
                string.enumeration.clear();
            }
        }
        SchemaKind::Type(Type::Object(object)) => {
            normalize_object_schema(object, path, warnings, stats)?
        }
        SchemaKind::Type(Type::Array(array)) => {
            normalize_array_schema(array, path, warnings, stats)?
        }
        SchemaKind::OneOf { one_of } => {
            normalize_schema_refs(one_of, &format!("{path}.oneOf"), warnings, stats)?
        }
        SchemaKind::AllOf { all_of } => {
            normalize_schema_refs(all_of, &format!("{path}.allOf"), warnings, stats)?
        }
        SchemaKind::AnyOf { any_of } => {
            normalize_schema_refs(any_of, &format!("{path}.anyOf"), warnings, stats)?
        }
        SchemaKind::Not { not } => {
            normalize_boxed_reference_or_schema(not, &format!("{path}.not"), warnings, stats)?
        }
        SchemaKind::Any(any) => {
            if let Some(typ) = any.typ.clone() {
                if !is_supported_schema_type(&typ) {
                    any.typ = None;
                    warnings.push(format!(
                        "normalized {path} — replaced unsupported type '{typ}' with fallback"
                    ));
                }
            }
            if let Some(colliding) = json_enum_collision(&any.enumeration) {
                warnings.push(format!(
                    "normalized {path} — dropped enum constraint (values [{colliding}] collide on Rust identifier sanitization); field is now a free-form string preserving wire format"
                ));
                any.enumeration.clear();
            }
            drop_colliding_properties(&mut any.properties, &mut any.required, path, warnings);
            for (name, property) in any.properties.iter_mut() {
                normalize_boxed_schema_ref(
                    property,
                    &format!("{path}.properties.{name}"),
                    warnings,
                    stats,
                )?;
            }
            if let Some(items) = any.items.as_mut() {
                normalize_boxed_schema_ref(items, &format!("{path}.items"), warnings, stats)?;
            }
            if let Some(openapiv3::AdditionalProperties::Schema(schema)) =
                any.additional_properties.as_mut()
            {
                normalize_boxed_reference_or_schema(
                    schema,
                    &format!("{path}.additionalProperties"),
                    warnings,
                    stats,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn normalize_object_schema(
    object: &mut ObjectType,
    path: &str,
    warnings: &mut Vec<String>,
    stats: &mut NormalizeStats,
) -> Result<()> {
    drop_colliding_properties(&mut object.properties, &mut object.required, path, warnings);
    for (name, property) in object.properties.iter_mut() {
        normalize_boxed_schema_ref(
            property,
            &format!("{path}.properties.{name}"),
            warnings,
            stats,
        )?;
    }
    if let Some(openapiv3::AdditionalProperties::Schema(schema)) =
        object.additional_properties.as_mut()
    {
        normalize_boxed_reference_or_schema(
            schema,
            &format!("{path}.additionalProperties"),
            warnings,
            stats,
        )?;
    }
    Ok(())
}

fn normalize_array_schema(
    array: &mut ArrayType,
    path: &str,
    warnings: &mut Vec<String>,
    stats: &mut NormalizeStats,
) -> Result<()> {
    if let Some(items) = array.items.as_mut() {
        normalize_boxed_schema_ref(items, &format!("{path}.items"), warnings, stats)?;
    }
    Ok(())
}

fn normalize_schema_refs(
    refs: &mut [ReferenceOr<Schema>],
    path: &str,
    warnings: &mut Vec<String>,
    stats: &mut NormalizeStats,
) -> Result<()> {
    for (i, schema) in refs.iter_mut().enumerate() {
        normalize_schema_ref(schema, &format!("{path}[{i}]"), warnings, stats)?;
    }
    Ok(())
}

fn normalize_schema_ref(
    schema: &mut ReferenceOr<Schema>,
    path: &str,
    warnings: &mut Vec<String>,
    stats: &mut NormalizeStats,
) -> Result<()> {
    if let ReferenceOr::Item(schema) = schema {
        normalize_schema(schema, path, warnings, stats)?;
    }
    Ok(())
}

fn normalize_boxed_schema_ref(
    schema: &mut ReferenceOr<Box<Schema>>,
    path: &str,
    warnings: &mut Vec<String>,
    stats: &mut NormalizeStats,
) -> Result<()> {
    if let ReferenceOr::Item(schema) = schema {
        normalize_schema(schema.as_mut(), path, warnings, stats)?;
    }
    Ok(())
}

fn normalize_boxed_reference_or_schema(
    schema: &mut Box<ReferenceOr<Schema>>,
    path: &str,
    warnings: &mut Vec<String>,
    stats: &mut NormalizeStats,
) -> Result<()> {
    normalize_schema_ref(schema.as_mut(), path, warnings, stats)
}

fn is_supported_schema_type(typ: &str) -> bool {
    matches!(
        typ,
        "string" | "number" | "integer" | "boolean" | "array" | "object"
    )
}

fn drop_colliding_properties<V>(
    properties: &mut indexmap::IndexMap<String, V>,
    required: &mut Vec<String>,
    path: &str,
    warnings: &mut Vec<String>,
) {
    let mut by_ident: HashMap<String, Vec<String>> = HashMap::new();
    for name in properties.keys() {
        by_ident
            .entry(enum_identifier_form(name))
            .or_default()
            .push(name.clone());
    }
    let mut to_drop: Vec<String> = Vec::new();
    for (_ident, names) in by_ident {
        if names.len() > 1 {
            let kept = &names[0];
            let dropped: Vec<String> = names.iter().skip(1).cloned().collect();
            warnings.push(format!(
                "normalized {path} — kept property '{kept}', dropped colliding [{}] (Rust identifier sanitization collision); wire format preserved for kept field",
                dropped.join(", ")
            ));
            to_drop.extend(dropped);
        }
    }
    for name in &to_drop {
        properties.shift_remove(name);
    }
    required.retain(|name| !to_drop.contains(name));
}

fn string_enum_collision(values: &[Option<String>]) -> Option<String> {
    let strings: Vec<&str> = values.iter().filter_map(Option::as_deref).collect();
    find_enum_collision(strings)
}

fn json_enum_collision(values: &[serde_json::Value]) -> Option<String> {
    let strings: Vec<&str> = values
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    find_enum_collision(strings)
}

fn find_enum_collision(strings: Vec<&str>) -> Option<String> {
    let mut by_ident: HashMap<String, Vec<&str>> = HashMap::new();
    for value in strings {
        by_ident
            .entry(enum_identifier_form(value))
            .or_default()
            .push(value);
    }
    by_ident
        .into_values()
        .find(|values| values.len() > 1)
        .map(|values| values.join(", "))
}

fn enum_identifier_form(value: &str) -> String {
    let mut out = String::new();
    let mut previous_underscore = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            previous_underscore = false;
        } else if !previous_underscore {
            out.push('_');
            previous_underscore = true;
        }
    }
    if out.is_empty() {
        return "_".to_string();
    }
    if out.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        out.insert_str(0, "n_");
    }
    out
}

fn request_body_has_schemaless_content(request_body: &RequestBody) -> bool {
    request_body
        .content
        .values()
        .any(|media_type| media_type.schema.is_none())
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

fn normalize_request_content(
    content: &mut indexmap::IndexMap<String, MediaType>,
) -> Option<(String, Vec<String>)> {
    let supported: Vec<String> = content
        .keys()
        .filter(|mime| is_supported_request_mime(mime))
        .cloned()
        .collect();
    if supported.is_empty() {
        content.clear();
        return None;
    }
    if supported.len() == content.len() && content.len() <= 1 {
        return None;
    }

    let kept = if content.contains_key(JSON_MIME) {
        JSON_MIME.to_string()
    } else {
        supported
            .into_iter()
            .min()
            .expect("supported content exists")
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

fn is_supported_request_mime(mime: &str) -> bool {
    matches!(mime, JSON_MIME | FORM_MIME | OCTET_STREAM_MIME)
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
    fn verbose_operation_ids_are_shortened() {
        assert_eq!(
            shorten_candidate("foo_bar_baz_qux_quux_widget_get").as_deref(),
            Some("quux_widget_get")
        );
        assert_eq!(
            shorten_candidate("PlausibleWeb.Plugins.API.Controllers.Capabilities.index").as_deref(),
            Some("capabilities_index")
        );
    }

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
