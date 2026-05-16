use anyhow::Result;
use openapiv3::{
    ArrayType, MediaType, ObjectType, OpenAPI, Operation, QueryStyle, ReferenceOr, RequestBody,
    Response, Schema, SchemaKind, StatusCode, Type,
};
use std::collections::{HashMap, HashSet};

use crate::spec::normalization_rules::{self as rules, typed};
use crate::spec::report::{ReportEntry, ReportSubject};
use crate::spec::traversal;

pub(super) const JSON_MIME: &str = "application/json";
pub(super) const FORM_MIME: &str = "application/x-www-form-urlencoded";
const OCTET_STREAM_MIME: &str = "application/octet-stream";

pub(super) fn apply(spec: &mut OpenAPI, reports: &mut Vec<ReportEntry>) -> Result<NormalizeStats> {
    let mut stats = NormalizeStats::default();
    normalize_components(spec, reports, &mut stats)?;
    let object_schema_refs = component_object_schema_refs(spec);
    normalize_operations(spec, reports, &mut stats, &object_schema_refs);
    Ok(stats)
}

fn normalize_components(
    spec: &mut OpenAPI,
    reports: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
) -> Result<()> {
    let Some(components) = spec.components.as_mut() else {
        return Ok(());
    };
    for (name, schema) in components.schemas.iter_mut() {
        if let ReferenceOr::Item(schema) = schema {
            normalize_schema(schema, &format!("component schema {name}"), reports, stats)?;
        }
    }
    for (name, request_body) in components.request_bodies.iter_mut() {
        if let ReferenceOr::Item(request_body) = request_body {
            normalize_request_body(
                request_body,
                &format!("component requestBody {name}"),
                reports,
                stats,
            );
        }
    }
    for (name, response) in components.responses.iter_mut() {
        if let ReferenceOr::Item(response) = response {
            normalize_response(
                response,
                &format!("component response {name}"),
                reports,
                stats,
            );
        }
    }
    Ok(())
}

fn normalize_operations(
    spec: &mut OpenAPI,
    reports: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    object_schema_refs: &HashSet<String>,
) {
    traversal::visit_operation_slots_mut(spec, |slot| {
        normalize_maybe_operation(
            slot.method_uppercase,
            slot.path,
            slot.operation,
            reports,
            stats,
            object_schema_refs,
        );
    });
}

pub(super) fn emit_summary_reports(reports: &mut Vec<ReportEntry>, stats: &NormalizeStats) {
    if stats.dropped_defaults > 0 {
        reports.push(rules::typed_warning(
            typed::SCHEMA_DEFAULTS_DROPPED,
            format!(
                "normalized {} schemas — dropped default values",
                stats.dropped_defaults
            ),
            None,
        ));
    }
    if !stats.dropped_unsupported_request_body_ops.is_empty() {
        reports.push(rules::typed_warning(
            typed::UNSUPPORTED_REQUEST_BODIES_DROPPED,
            format!(
                "dropped {} operations with progenitor-unsupported request body: {}",
                stats.dropped_unsupported_request_body_ops.len(),
                stats.dropped_unsupported_request_body_ops.join(", ")
            ),
            None,
        ));
    }
    if !stats.normalized_deep_object_query_params.is_empty() {
        reports.push(rules::typed_warning(
            typed::DEEP_OBJECT_QUERY_PARAMS_REWRITTEN,
            format!(
                "normalized {} query parameters — replaced unsupported deepObject style with form: {}",
                stats.normalized_deep_object_query_params.len(),
                stats.normalized_deep_object_query_params.join(", ")
            ),
            None,
        ));
    }
}

pub(super) fn emit_optional_object_query_param_report(
    reports: &mut Vec<ReportEntry>,
    stats: &NormalizeStats,
) {
    if !stats.dropped_optional_object_query_params.is_empty() {
        reports.push(rules::typed_warning(
            typed::OPTIONAL_OBJECT_QUERY_PARAMS_DROPPED,
            format!(
                "dropped {} optional object query parameters with progenitor-unsupported builder shape: {}",
                stats.dropped_optional_object_query_params.len(),
                stats.dropped_optional_object_query_params.join(", ")
            ),
            None,
        ));
    }
}

#[derive(Default)]
pub(super) struct NormalizeStats {
    dropped_defaults: usize,
    dropped_unsupported_request_body_ops: Vec<String>,
    normalized_deep_object_query_params: Vec<String>,
    dropped_optional_object_query_params: Vec<String>,
}

fn component_object_schema_refs(spec: &OpenAPI) -> HashSet<String> {
    let mut refs = HashSet::new();
    let Some(components) = spec.components.as_ref() else {
        return refs;
    };

    for (name, schema) in &components.schemas {
        if let ReferenceOr::Item(schema) = schema {
            if schema_is_object_shaped(schema) {
                refs.insert(format!("#/components/schemas/{name}"));
            }
        }
    }
    refs
}

fn schema_is_object_shaped(schema: &Schema) -> bool {
    match &schema.schema_kind {
        SchemaKind::Type(Type::Object(_)) => true,
        SchemaKind::Any(any) => !any.properties.is_empty(),
        SchemaKind::AllOf { all_of } | SchemaKind::OneOf { one_of: all_of } => all_of.iter().any(
            |schema| matches!(schema, ReferenceOr::Item(schema) if schema_is_object_shaped(schema)),
        ),
        SchemaKind::AnyOf { any_of } => any_of.iter().any(
            |schema| matches!(schema, ReferenceOr::Item(schema) if schema_is_object_shaped(schema)),
        ),
        _ => false,
    }
}

fn optional_object_query_param_name(
    param: &openapiv3::Parameter,
    object_schema_refs: &HashSet<String>,
) -> Option<String> {
    let openapiv3::Parameter::Query { parameter_data, .. } = param else {
        return None;
    };
    if parameter_data.required {
        return None;
    }
    let openapiv3::ParameterSchemaOrContent::Schema(schema) = &parameter_data.format else {
        return None;
    };

    let is_object = match schema {
        ReferenceOr::Item(schema) => schema_is_object_shaped(schema),
        ReferenceOr::Reference { reference } => object_schema_refs.contains(reference),
    };
    is_object.then(|| parameter_data.name.clone())
}

fn normalize_maybe_operation(
    method: &str,
    path: &str,
    operation: &mut Option<Operation>,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    object_schema_refs: &HashSet<String>,
) {
    let Some(operation_ref) = operation.as_mut() else {
        return;
    };
    let op_name = operation_name(method, path, operation_ref);
    if normalize_operation(operation_ref, &op_name, warnings, stats, object_schema_refs) {
        stats.dropped_unsupported_request_body_ops.push(op_name);
        *operation = None;
    }
}

fn normalize_operation(
    operation: &mut Operation,
    op_name: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    object_schema_refs: &HashSet<String>,
) -> bool {
    normalize_response_variants(operation, op_name, warnings);

    let mut dropped_param_indices = Vec::new();
    for (i, param) in operation.parameters.iter_mut().enumerate() {
        if let ReferenceOr::Item(param) = param {
            if let Some(param_name) = optional_object_query_param_name(param, object_schema_refs) {
                stats
                    .dropped_optional_object_query_params
                    .push(format!("{op_name}.{param_name}"));
                dropped_param_indices.push(i);
                continue;
            }

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
    for i in dropped_param_indices.into_iter().rev() {
        operation.parameters.remove(i);
    }

    if let Some(ReferenceOr::Item(request_body)) = operation.request_body.as_mut() {
        if normalize_request_body(request_body, op_name, warnings, stats) {
            return true;
        }
        if request_body_has_schemaless_content(request_body) {
            operation.request_body = None;
            warnings.push(rules::typed_warning(
                typed::SCHEMALESS_REQUEST_BODY_DROPPED,
                format!("normalized {op_name} — dropped requestBody (no schema specified)"),
                Some(ReportSubject::operation(op_name)),
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
    warnings: &mut Vec<ReportEntry>,
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

    warnings.push(rules::typed_warning(
        typed::RESPONSE_VARIANTS_PRUNED,
        format!(
            "normalized {op_name} responses — kept {kept}, dropped {}",
            dropped.join(", ")
        ),
        Some(ReportSubject::operation(op_name)),
    ));
}

fn normalize_request_body(
    request_body: &mut RequestBody,
    op_name: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
) -> bool {
    if let Some((kept, dropped)) = normalize_request_content(&mut request_body.content) {
        warnings.push(rules::typed_warning(
            typed::CONTENT_TYPES_PRUNED,
            format!(
                "normalized {op_name} — kept {kept}, dropped {}",
                dropped.join(", ")
            ),
            Some(ReportSubject::operation(op_name)),
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
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
) {
    if let Some((kept, dropped)) = normalize_content(&mut response.content) {
        warnings.push(rules::typed_warning(
            typed::CONTENT_TYPES_PRUNED,
            format!(
                "normalized {op_name} — kept {kept}, dropped {}",
                dropped.join(", ")
            ),
            Some(ReportSubject::operation(op_name)),
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
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
) -> Result<()> {
    if schema.schema_data.default.take().is_some() {
        stats.dropped_defaults += 1;
    }

    match &mut schema.schema_kind {
        SchemaKind::Type(Type::String(string)) => {
            if let Some(colliding) = string_enum_collision(&string.enumeration) {
                warnings.push(rules::typed_warning(
                    typed::ENUM_CONSTRAINT_DROPPED,
                    format!("normalized {path} — dropped enum constraint (values [{colliding}] collide on Rust identifier sanitization); field is now a free-form string preserving wire format"),
                    Some(ReportSubject::schema(path)),
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
                    warnings.push(rules::typed_warning(
                        typed::UNSUPPORTED_SCHEMA_TYPE_REPLACED,
                        format!(
                            "normalized {path} — replaced unsupported type '{typ}' with fallback"
                        ),
                        Some(ReportSubject::schema(path)),
                    ));
                }
            }
            if let Some(colliding) = json_enum_collision(&any.enumeration) {
                warnings.push(rules::typed_warning(
                    typed::ENUM_CONSTRAINT_DROPPED,
                    format!("normalized {path} — dropped enum constraint (values [{colliding}] collide on Rust identifier sanitization); field is now a free-form string preserving wire format"),
                    Some(ReportSubject::schema(path)),
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
    warnings: &mut Vec<ReportEntry>,
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
    warnings: &mut Vec<ReportEntry>,
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
    warnings: &mut Vec<ReportEntry>,
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
    warnings: &mut Vec<ReportEntry>,
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
    warnings: &mut Vec<ReportEntry>,
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
    warnings: &mut Vec<ReportEntry>,
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
    warnings: &mut Vec<ReportEntry>,
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
            warnings.push(rules::typed_warning(
                typed::PROPERTIES_COLLIDING_DROPPED,
                format!(
                    "normalized {path} — kept property '{kept}', dropped colliding [{}] (Rust identifier sanitization collision); wire format preserved for kept field",
                    dropped.join(", ")
                ),
                Some(ReportSubject::schema(path)),
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
    traversal::operation_identifier(method, path, operation)
}
