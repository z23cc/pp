use anyhow::Result;
use openapiv3::{ArrayType, ObjectType, ReferenceOr, Schema, SchemaKind, Type};
use std::collections::{HashMap, HashSet};

use super::{
    actions::{
        CompatibilityAggregateProposal, CompatibilityTransformAction, CompatibilityTransformPlan,
    },
    schema_defaults, NormalizeStats,
};
use crate::backend::BackendCapabilities;
use crate::spec::normalization_rules::{self as rules, typed};
use crate::spec::report::{ReportEntry, ReportSubject};

fn enum_constraint_report(path: &str, colliding: &str) -> ReportEntry {
    rules::typed_warning(
        typed::ENUM_CONSTRAINT_DROPPED,
        format!("normalized {path} — dropped enum constraint (values [{colliding}] collide on Rust identifier sanitization); field is now a free-form string preserving wire format"),
        Some(ReportSubject::schema(path)),
    )
}

fn unsupported_schema_type_report(path: &str, typ: &str) -> ReportEntry {
    rules::typed_warning(
        typed::UNSUPPORTED_SCHEMA_TYPE_REPLACED,
        format!("normalized {path} — replaced unsupported type '{typ}' with fallback"),
        Some(ReportSubject::schema(path)),
    )
}

fn colliding_properties_report(path: &str, kept: &str, dropped: &[String]) -> ReportEntry {
    rules::typed_warning(
        typed::PROPERTIES_COLLIDING_DROPPED,
        format!(
            "normalized {path} — kept property '{kept}', dropped colliding [{}] (Rust identifier sanitization collision); wire format preserved for kept field",
            dropped.join(", ")
        ),
        Some(ReportSubject::schema(path)),
    )
}
pub(super) fn schema_is_object_shaped(schema: &Schema) -> bool {
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
pub(super) fn propose_schema_transforms(
    schema: &Schema,
    path: &str,
    plan: &mut CompatibilityTransformPlan,
    aggregate: &mut CompatibilityAggregateProposal,
    backend_capabilities: &BackendCapabilities,
) {
    if schema_defaults::should_propose(schema, backend_capabilities) {
        aggregate.schema_defaults.push(path.to_string());
    }

    match &schema.schema_kind {
        SchemaKind::Type(Type::String(string))
            if backend_capabilities
                .schemas
                .requires_unique_sanitized_enum_variants =>
        {
            if let Some(colliding) = string_enum_collision(&string.enumeration) {
                plan.push(CompatibilityTransformAction::DropEnumConstraint {
                    target: path.to_string(),
                    report: enum_constraint_report(path, &colliding),
                });
            }
        }
        SchemaKind::Type(Type::Object(object)) => {
            propose_object_schema_transforms(object, path, plan, aggregate, backend_capabilities);
        }
        SchemaKind::Type(Type::Array(array)) => {
            if let Some(items) = array.items.as_ref() {
                propose_boxed_schema_ref_transforms(
                    items,
                    &format!("{path}.items"),
                    plan,
                    aggregate,
                    backend_capabilities,
                );
            }
        }
        SchemaKind::OneOf { one_of } => propose_schema_ref_transforms(
            one_of,
            &format!("{path}.oneOf"),
            plan,
            aggregate,
            backend_capabilities,
        ),
        SchemaKind::AllOf { all_of } => propose_schema_ref_transforms(
            all_of,
            &format!("{path}.allOf"),
            plan,
            aggregate,
            backend_capabilities,
        ),
        SchemaKind::AnyOf { any_of } => propose_schema_ref_transforms(
            any_of,
            &format!("{path}.anyOf"),
            plan,
            aggregate,
            backend_capabilities,
        ),
        SchemaKind::Not { not } => propose_reference_or_schema_transforms(
            not.as_ref(),
            &format!("{path}.not"),
            plan,
            aggregate,
            backend_capabilities,
        ),
        SchemaKind::Any(any) => {
            if let Some(typ) = any.typ.as_ref() {
                if !is_supported_schema_type(typ, backend_capabilities) {
                    plan.push(CompatibilityTransformAction::ReplaceUnsupportedSchemaType {
                        target: path.to_string(),
                        report: unsupported_schema_type_report(path, typ),
                    });
                }
            }
            if backend_capabilities
                .schemas
                .requires_unique_sanitized_enum_variants
            {
                if let Some(colliding) = json_enum_collision(&any.enumeration) {
                    plan.push(CompatibilityTransformAction::DropEnumConstraint {
                        target: path.to_string(),
                        report: enum_constraint_report(path, &colliding),
                    });
                }
            }
            let dropped = if backend_capabilities
                .schemas
                .requires_unique_sanitized_object_properties
            {
                propose_colliding_property_actions(&any.properties, path, plan)
            } else {
                HashSet::new()
            };
            for (name, property) in &any.properties {
                if dropped.contains(name) {
                    continue;
                }
                propose_boxed_schema_ref_transforms(
                    property,
                    &format!("{path}.properties.{name}"),
                    plan,
                    aggregate,
                    backend_capabilities,
                );
            }
            if let Some(items) = any.items.as_ref() {
                propose_boxed_schema_ref_transforms(
                    items,
                    &format!("{path}.items"),
                    plan,
                    aggregate,
                    backend_capabilities,
                );
            }
            if let Some(openapiv3::AdditionalProperties::Schema(schema)) =
                any.additional_properties.as_ref()
            {
                propose_reference_or_schema_transforms(
                    schema.as_ref(),
                    &format!("{path}.additionalProperties"),
                    plan,
                    aggregate,
                    backend_capabilities,
                );
            }
        }
        _ => {}
    }
}

fn propose_object_schema_transforms(
    object: &ObjectType,
    path: &str,
    plan: &mut CompatibilityTransformPlan,
    aggregate: &mut CompatibilityAggregateProposal,
    backend_capabilities: &BackendCapabilities,
) {
    let dropped = if backend_capabilities
        .schemas
        .requires_unique_sanitized_object_properties
    {
        propose_colliding_property_actions(&object.properties, path, plan)
    } else {
        HashSet::new()
    };
    for (name, property) in &object.properties {
        if dropped.contains(name) {
            continue;
        }
        propose_boxed_schema_ref_transforms(
            property,
            &format!("{path}.properties.{name}"),
            plan,
            aggregate,
            backend_capabilities,
        );
    }
    if let Some(openapiv3::AdditionalProperties::Schema(schema)) =
        object.additional_properties.as_ref()
    {
        propose_reference_or_schema_transforms(
            schema.as_ref(),
            &format!("{path}.additionalProperties"),
            plan,
            aggregate,
            backend_capabilities,
        );
    }
}

fn propose_schema_ref_transforms(
    refs: &[ReferenceOr<Schema>],
    path: &str,
    plan: &mut CompatibilityTransformPlan,
    aggregate: &mut CompatibilityAggregateProposal,
    backend_capabilities: &BackendCapabilities,
) {
    for (i, schema) in refs.iter().enumerate() {
        propose_reference_or_schema_transforms(
            schema,
            &format!("{path}[{i}]"),
            plan,
            aggregate,
            backend_capabilities,
        );
    }
}

fn propose_reference_or_schema_transforms(
    schema: &ReferenceOr<Schema>,
    path: &str,
    plan: &mut CompatibilityTransformPlan,
    aggregate: &mut CompatibilityAggregateProposal,
    backend_capabilities: &BackendCapabilities,
) {
    if let ReferenceOr::Item(schema) = schema {
        propose_schema_transforms(schema, path, plan, aggregate, backend_capabilities);
    }
}

fn propose_boxed_schema_ref_transforms(
    schema: &ReferenceOr<Box<Schema>>,
    path: &str,
    plan: &mut CompatibilityTransformPlan,
    aggregate: &mut CompatibilityAggregateProposal,
    backend_capabilities: &BackendCapabilities,
) {
    if let ReferenceOr::Item(schema) = schema {
        propose_schema_transforms(schema.as_ref(), path, plan, aggregate, backend_capabilities);
    }
}

fn propose_colliding_property_actions<V>(
    properties: &indexmap::IndexMap<String, V>,
    path: &str,
    plan: &mut CompatibilityTransformPlan,
) -> HashSet<String> {
    let mut dropped_names = HashSet::new();
    for (kept, dropped) in colliding_properties(properties) {
        dropped_names.extend(dropped.iter().cloned());
        plan.push(CompatibilityTransformAction::DropCollidingProperties {
            target: path.to_string(),
            report: colliding_properties_report(path, &kept, &dropped),
            dropped,
        });
    }
    dropped_names
}

pub(super) fn normalize_schema(
    schema: &mut Schema,
    path: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    approved_transforms: &CompatibilityTransformPlan,
) -> Result<()> {
    if schema_defaults::apply(schema, approved_transforms.should_drop_schema_default(path)) {
        stats.dropped_schema_defaults.push(path.to_string());
    }

    match &mut schema.schema_kind {
        SchemaKind::Type(Type::String(string)) => {
            if let Some(report) = approved_transforms.enum_constraint_for(path) {
                warnings.push(report.clone());
                string.enumeration.clear();
            }
        }
        SchemaKind::Type(Type::Object(object)) => {
            normalize_object_schema(object, path, warnings, stats, approved_transforms)?
        }
        SchemaKind::Type(Type::Array(array)) => {
            normalize_array_schema(array, path, warnings, stats, approved_transforms)?
        }
        SchemaKind::OneOf { one_of } => normalize_schema_refs(
            one_of,
            &format!("{path}.oneOf"),
            warnings,
            stats,
            approved_transforms,
        )?,
        SchemaKind::AllOf { all_of } => normalize_schema_refs(
            all_of,
            &format!("{path}.allOf"),
            warnings,
            stats,
            approved_transforms,
        )?,
        SchemaKind::AnyOf { any_of } => normalize_schema_refs(
            any_of,
            &format!("{path}.anyOf"),
            warnings,
            stats,
            approved_transforms,
        )?,
        SchemaKind::Not { not } => normalize_boxed_reference_or_schema(
            not,
            &format!("{path}.not"),
            warnings,
            stats,
            approved_transforms,
        )?,
        SchemaKind::Any(any) => {
            if let Some(report) = approved_transforms.unsupported_schema_type_for(path) {
                any.typ = None;
                warnings.push(report.clone());
            }
            if let Some(report) = approved_transforms.enum_constraint_for(path) {
                warnings.push(report.clone());
                any.enumeration.clear();
            }
            drop_colliding_properties(
                &mut any.properties,
                &mut any.required,
                path,
                warnings,
                approved_transforms,
            );
            for (name, property) in any.properties.iter_mut() {
                normalize_boxed_schema_ref(
                    property,
                    &format!("{path}.properties.{name}"),
                    warnings,
                    stats,
                    approved_transforms,
                )?;
            }
            if let Some(items) = any.items.as_mut() {
                normalize_boxed_schema_ref(
                    items,
                    &format!("{path}.items"),
                    warnings,
                    stats,
                    approved_transforms,
                )?;
            }
            if let Some(openapiv3::AdditionalProperties::Schema(schema)) =
                any.additional_properties.as_mut()
            {
                normalize_boxed_reference_or_schema(
                    schema,
                    &format!("{path}.additionalProperties"),
                    warnings,
                    stats,
                    approved_transforms,
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
    approved_transforms: &CompatibilityTransformPlan,
) -> Result<()> {
    drop_colliding_properties(
        &mut object.properties,
        &mut object.required,
        path,
        warnings,
        approved_transforms,
    );
    for (name, property) in object.properties.iter_mut() {
        normalize_boxed_schema_ref(
            property,
            &format!("{path}.properties.{name}"),
            warnings,
            stats,
            approved_transforms,
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
            approved_transforms,
        )?;
    }
    Ok(())
}

fn normalize_array_schema(
    array: &mut ArrayType,
    path: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    approved_transforms: &CompatibilityTransformPlan,
) -> Result<()> {
    if let Some(items) = array.items.as_mut() {
        normalize_boxed_schema_ref(
            items,
            &format!("{path}.items"),
            warnings,
            stats,
            approved_transforms,
        )?;
    }
    Ok(())
}

fn normalize_schema_refs(
    refs: &mut [ReferenceOr<Schema>],
    path: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    approved_transforms: &CompatibilityTransformPlan,
) -> Result<()> {
    for (i, schema) in refs.iter_mut().enumerate() {
        normalize_schema_ref(
            schema,
            &format!("{path}[{i}]"),
            warnings,
            stats,
            approved_transforms,
        )?;
    }
    Ok(())
}

fn normalize_schema_ref(
    schema: &mut ReferenceOr<Schema>,
    path: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    approved_transforms: &CompatibilityTransformPlan,
) -> Result<()> {
    if let ReferenceOr::Item(schema) = schema {
        normalize_schema(schema, path, warnings, stats, approved_transforms)?;
    }
    Ok(())
}

fn normalize_boxed_schema_ref(
    schema: &mut ReferenceOr<Box<Schema>>,
    path: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    approved_transforms: &CompatibilityTransformPlan,
) -> Result<()> {
    if let ReferenceOr::Item(schema) = schema {
        normalize_schema(schema.as_mut(), path, warnings, stats, approved_transforms)?;
    }
    Ok(())
}

fn normalize_boxed_reference_or_schema(
    schema: &mut Box<ReferenceOr<Schema>>,
    path: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    approved_transforms: &CompatibilityTransformPlan,
) -> Result<()> {
    normalize_schema_ref(schema.as_mut(), path, warnings, stats, approved_transforms)
}

fn is_supported_schema_type(typ: &str, backend_capabilities: &BackendCapabilities) -> bool {
    backend_capabilities.schemas.supported_types.contains(&typ)
}

fn drop_colliding_properties<V>(
    properties: &mut indexmap::IndexMap<String, V>,
    required: &mut Vec<String>,
    path: &str,
    warnings: &mut Vec<ReportEntry>,
    approved_transforms: &CompatibilityTransformPlan,
) {
    let mut to_drop: Vec<String> = Vec::new();
    for action in approved_transforms.colliding_properties_for(path) {
        if let CompatibilityTransformAction::DropCollidingProperties {
            dropped, report, ..
        } = action
        {
            warnings.push(report.clone());
            to_drop.extend(dropped.iter().cloned());
        }
    }
    for name in &to_drop {
        properties.shift_remove(name);
    }
    required.retain(|name| !to_drop.contains(name));
}

fn colliding_properties<V>(
    properties: &indexmap::IndexMap<String, V>,
) -> Vec<(String, Vec<String>)> {
    let mut by_ident: HashMap<String, Vec<String>> = HashMap::new();
    for name in properties.keys() {
        by_ident
            .entry(enum_identifier_form(name))
            .or_default()
            .push(name.clone());
    }
    by_ident
        .into_values()
        .filter_map(|names| {
            (names.len() > 1).then(|| {
                let kept = names[0].clone();
                let dropped = names.into_iter().skip(1).collect();
                (kept, dropped)
            })
        })
        .collect()
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
