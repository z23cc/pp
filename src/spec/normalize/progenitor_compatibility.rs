use anyhow::Result;
use openapiv3::{
    ArrayType, MediaType, ObjectType, OpenAPI, Operation, QueryStyle, ReferenceOr, RequestBody,
    Response, Schema, SchemaKind, StatusCode, Type,
};
use std::collections::{HashMap, HashSet};

use crate::backend::BackendCapabilities;
use crate::spec::normalization_rules::{self as rules, typed};
use crate::spec::report::{ReportEntry, ReportSubject};
use crate::spec::traversal;

pub(super) const JSON_MIME: &str = "application/json";
#[cfg(test)]
pub(super) const FORM_MIME: &str = "application/x-www-form-urlencoded";

pub(crate) fn propose_transforms(
    spec: &OpenAPI,
    backend_capabilities: &BackendCapabilities,
) -> CompatibilityTransformPlan {
    let mut plan = CompatibilityTransformPlan::default();
    propose_component_transforms(spec, &mut plan, backend_capabilities);
    propose_operation_transforms(spec, &mut plan, backend_capabilities);
    plan
}

pub(super) fn apply_approved(
    spec: &mut OpenAPI,
    reports: &mut Vec<ReportEntry>,
    backend_capabilities: &BackendCapabilities,
    approved_transforms: &CompatibilityTransformPlan,
) -> Result<NormalizeStats> {
    let mut stats = NormalizeStats::default();
    normalize_components(
        spec,
        reports,
        &mut stats,
        backend_capabilities,
        approved_transforms,
    )?;
    let object_schema_refs = component_object_schema_refs(spec);
    normalize_operations(
        spec,
        reports,
        &mut stats,
        &object_schema_refs,
        backend_capabilities,
        approved_transforms,
    )?;
    Ok(stats)
}

fn normalize_components(
    spec: &mut OpenAPI,
    reports: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    backend_capabilities: &BackendCapabilities,
    approved_transforms: &CompatibilityTransformPlan,
) -> Result<()> {
    let Some(components) = spec.components.as_mut() else {
        return Ok(());
    };
    for (name, schema) in components.schemas.iter_mut() {
        if let ReferenceOr::Item(schema) = schema {
            normalize_schema(
                schema,
                &format!("component schema {name}"),
                reports,
                stats,
                backend_capabilities,
            )?;
        }
    }
    let mut dropped_request_bodies = Vec::new();
    for (name, request_body) in components.request_bodies.iter_mut() {
        if let ReferenceOr::Item(request_body) = request_body {
            if normalize_request_body(
                request_body,
                &format!("component requestBody {name}"),
                reports,
                stats,
                backend_capabilities,
                approved_transforms,
                &ContentTarget::ComponentRequestBody(name.clone()),
            ) {
                dropped_request_bodies.push(name.clone());
            }
        }
    }
    for name in dropped_request_bodies {
        components.request_bodies.shift_remove(&name);
    }
    for (name, response) in components.responses.iter_mut() {
        if let ReferenceOr::Item(response) = response {
            normalize_response(
                response,
                &format!("component response {name}"),
                reports,
                stats,
                backend_capabilities,
                approved_transforms,
                &ContentTarget::ComponentResponse(name.clone()),
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
    backend_capabilities: &BackendCapabilities,
    approved_transforms: &CompatibilityTransformPlan,
) -> Result<()> {
    traversal::visit_operation_slots_mut(spec, |slot| {
        normalize_maybe_operation(
            slot.method_uppercase,
            slot.path,
            slot.operation,
            reports,
            stats,
            object_schema_refs,
            backend_capabilities,
            approved_transforms,
        );
    });
    Ok(())
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

#[derive(Debug, Clone, Default)]
pub(crate) struct CompatibilityTransformPlan {
    actions: Vec<CompatibilityTransformAction>,
}

impl CompatibilityTransformPlan {
    pub(crate) fn report_entries(&self) -> Vec<ReportEntry> {
        self.actions
            .iter()
            .map(CompatibilityTransformAction::report_entry)
            .cloned()
            .collect()
    }

    fn push(&mut self, action: CompatibilityTransformAction) {
        self.actions.push(action);
    }

    fn response_variants_for(
        &self,
        method: &str,
        path: &str,
    ) -> Option<&CompatibilityTransformAction> {
        self.actions.iter().find(|action| {
            matches!(
                action,
                CompatibilityTransformAction::PruneResponseVariants { target, .. }
                    if target.method == method && target.path == path
            )
        })
    }

    fn content_for(&self, target: &ContentTarget) -> Option<&CompatibilityTransformAction> {
        self.actions.iter().find(|action| {
            matches!(
                action,
                CompatibilityTransformAction::PruneContentTypes { target: action_target, .. }
                    if action_target == target
            )
        })
    }

    fn unsupported_request_body_for(
        &self,
        target: &ContentTarget,
    ) -> Option<&CompatibilityTransformAction> {
        self.actions.iter().find(|action| {
            matches!(
                action,
                CompatibilityTransformAction::DropUnsupportedRequestBody { target: action_target, .. }
                    if action_target == target
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OperationTarget {
    method: String,
    path: String,
}

impl OperationTarget {
    fn new(method: &str, path: &str) -> Self {
        Self {
            method: method.to_string(),
            path: path.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ContentTarget {
    ComponentRequestBody(String),
    ComponentResponse(String),
    OperationRequestBody(OperationTarget),
    OperationResponse {
        operation: OperationTarget,
        status: String,
    },
    OperationDefaultResponse(OperationTarget),
}

#[derive(Debug, Clone)]
enum CompatibilityTransformAction {
    PruneResponseVariants {
        target: OperationTarget,
        kept: String,
        dropped: Vec<String>,
        report: ReportEntry,
    },
    PruneContentTypes {
        target: ContentTarget,
        kept: String,
        dropped: Vec<String>,
        report: ReportEntry,
    },
    DropUnsupportedRequestBody {
        target: ContentTarget,
        report: ReportEntry,
    },
}

impl CompatibilityTransformAction {
    fn report_entry(&self) -> &ReportEntry {
        match self {
            Self::PruneResponseVariants { report, .. }
            | Self::PruneContentTypes { report, .. }
            | Self::DropUnsupportedRequestBody { report, .. } => report,
        }
    }
}

fn propose_component_transforms(
    spec: &OpenAPI,
    plan: &mut CompatibilityTransformPlan,
    backend_capabilities: &BackendCapabilities,
) {
    let Some(components) = spec.components.as_ref() else {
        return;
    };
    for (name, request_body) in &components.request_bodies {
        if let ReferenceOr::Item(request_body) = request_body {
            propose_request_content_transform(
                &request_body.content,
                &format!("component requestBody {name}"),
                ContentTarget::ComponentRequestBody(name.clone()),
                plan,
                backend_capabilities,
            );
        }
    }
    for (name, response) in &components.responses {
        if let ReferenceOr::Item(response) = response {
            propose_response_content_transform(
                &response.content,
                &format!("component response {name}"),
                ContentTarget::ComponentResponse(name.clone()),
                plan,
                backend_capabilities,
            );
        }
    }
}

fn propose_operation_transforms(
    spec: &OpenAPI,
    plan: &mut CompatibilityTransformPlan,
    backend_capabilities: &BackendCapabilities,
) {
    for operation_ref in traversal::operations(spec) {
        let op_name = operation_name(
            operation_ref.method_uppercase,
            operation_ref.path,
            operation_ref.operation,
        );
        let target = OperationTarget::new(operation_ref.method_uppercase, operation_ref.path);
        let kept_response = propose_response_variants_transform(
            operation_ref.operation,
            &op_name,
            target.clone(),
            plan,
            backend_capabilities,
        );

        if let Some(request_body) = operation_ref.operation.request_body.as_ref() {
            match request_body {
                ReferenceOr::Item(request_body) => propose_request_content_transform(
                    &request_body.content,
                    &op_name,
                    ContentTarget::OperationRequestBody(target.clone()),
                    plan,
                    backend_capabilities,
                ),
                ReferenceOr::Reference { reference } => {
                    if let Some(content) = component_request_body_content(spec, reference) {
                        if content_has_only_unsupported_request_types(content, backend_capabilities)
                        {
                            let target = ContentTarget::OperationRequestBody(target.clone());
                            plan.push(CompatibilityTransformAction::DropUnsupportedRequestBody {
                                report: unsupported_request_body_report(&op_name, &target, content),
                                target,
                            });
                        }
                    }
                }
            }
        }

        for (code, response) in &operation_ref.operation.responses.responses {
            let status = code.to_string();
            if kept_response.as_ref().is_some_and(|kept| kept != &status) {
                continue;
            }
            if let ReferenceOr::Item(response) = response {
                propose_response_content_transform(
                    &response.content,
                    &op_name,
                    ContentTarget::OperationResponse {
                        operation: target.clone(),
                        status,
                    },
                    plan,
                    backend_capabilities,
                );
            }
        }
        if kept_response
            .as_deref()
            .map_or(true, |kept| kept == "default")
        {
            if let Some(ReferenceOr::Item(response)) =
                operation_ref.operation.responses.default.as_ref()
            {
                propose_response_content_transform(
                    &response.content,
                    &op_name,
                    ContentTarget::OperationDefaultResponse(target),
                    plan,
                    backend_capabilities,
                );
            }
        }
    }
}

fn propose_response_variants_transform(
    operation: &Operation,
    op_name: &str,
    target: OperationTarget,
    plan: &mut CompatibilityTransformPlan,
    backend_capabilities: &BackendCapabilities,
) -> Option<String> {
    let (kept, dropped) = propose_response_variant_pruning(operation, backend_capabilities)?;
    plan.push(CompatibilityTransformAction::PruneResponseVariants {
        target,
        report: response_variants_report(op_name, &kept, &dropped),
        kept: kept.clone(),
        dropped,
    });
    Some(kept)
}

fn propose_request_content_transform(
    content: &indexmap::IndexMap<String, MediaType>,
    op_name: &str,
    target: ContentTarget,
    plan: &mut CompatibilityTransformPlan,
    backend_capabilities: &BackendCapabilities,
) {
    if content_has_only_unsupported_request_types(content, backend_capabilities) {
        plan.push(CompatibilityTransformAction::DropUnsupportedRequestBody {
            report: unsupported_request_body_report(op_name, &target, content),
            target,
        });
        return;
    }

    if let Some((kept, dropped)) = propose_request_content_pruning(content, backend_capabilities) {
        plan.push(CompatibilityTransformAction::PruneContentTypes {
            report: content_types_report(op_name, &target, &kept, &dropped),
            target,
            kept,
            dropped,
        });
    }
}

fn propose_response_content_transform(
    content: &indexmap::IndexMap<String, MediaType>,
    op_name: &str,
    target: ContentTarget,
    plan: &mut CompatibilityTransformPlan,
    backend_capabilities: &BackendCapabilities,
) {
    if let Some((kept, dropped)) = propose_response_content_pruning(content, backend_capabilities) {
        plan.push(CompatibilityTransformAction::PruneContentTypes {
            report: content_types_report(op_name, &target, &kept, &dropped),
            target,
            kept,
            dropped,
        });
    }
}

fn propose_response_variant_pruning(
    operation: &Operation,
    backend_capabilities: &BackendCapabilities,
) -> Option<(String, Vec<String>)> {
    let mut codes: Vec<String> = operation
        .responses
        .responses
        .keys()
        .map(ToString::to_string)
        .collect();
    if operation.responses.default.is_some() {
        codes.push("default".to_string());
    }
    if !backend_capabilities.requires_single_response_variant_per_operation || codes.len() <= 1 {
        return None;
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
    Some((kept, dropped))
}

fn response_variants_report(op_name: &str, kept: &str, dropped: &[String]) -> ReportEntry {
    rules::typed_warning(
        typed::RESPONSE_VARIANTS_PRUNED,
        format!(
            "normalized {op_name} responses — kept {kept}, dropped {}",
            dropped.join(", ")
        ),
        Some(ReportSubject::operation(op_name)),
    )
}

fn content_types_report(
    target_label: &str,
    target: &ContentTarget,
    kept: &str,
    dropped: &[String],
) -> ReportEntry {
    rules::typed_warning(
        typed::CONTENT_TYPES_PRUNED,
        format!(
            "normalized {target_label} — kept {kept}, dropped {}",
            dropped.join(", ")
        ),
        Some(content_report_subject(target, target_label)),
    )
}

fn unsupported_request_body_report(
    target_label: &str,
    target: &ContentTarget,
    content: &indexmap::IndexMap<String, MediaType>,
) -> ReportEntry {
    let dropped = content.keys().cloned().collect::<Vec<_>>().join(", ");
    rules::typed_warning(
        typed::UNSUPPORTED_REQUEST_BODIES_DROPPED,
        format!(
            "normalized {target_label} — dropped requestBody with only unsupported content types: {dropped}"
        ),
        Some(content_report_subject(target, target_label)),
    )
}

fn content_report_subject(target: &ContentTarget, target_label: &str) -> ReportSubject {
    match target {
        ContentTarget::ComponentRequestBody(_) | ContentTarget::ComponentResponse(_) => {
            ReportSubject::component(target_label)
        }
        ContentTarget::OperationRequestBody(_)
        | ContentTarget::OperationResponse { .. }
        | ContentTarget::OperationDefaultResponse(_) => ReportSubject::operation(target_label),
    }
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

fn component_request_body_content<'a>(
    spec: &'a OpenAPI,
    reference: &str,
) -> Option<&'a indexmap::IndexMap<String, MediaType>> {
    let name = reference.strip_prefix("#/components/requestBodies/")?;
    let components = spec.components.as_ref()?;
    let ReferenceOr::Item(request_body) = components.request_bodies.get(name)? else {
        return None;
    };
    Some(&request_body.content)
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

#[allow(clippy::too_many_arguments)]
fn normalize_maybe_operation(
    method: &str,
    path: &str,
    operation: &mut Option<Operation>,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    object_schema_refs: &HashSet<String>,
    backend_capabilities: &BackendCapabilities,
    approved_transforms: &CompatibilityTransformPlan,
) {
    let Some(operation_ref) = operation.as_mut() else {
        return;
    };
    let op_name = operation_name(method, path, operation_ref);
    if normalize_operation(
        method,
        path,
        operation_ref,
        &op_name,
        warnings,
        stats,
        object_schema_refs,
        backend_capabilities,
        approved_transforms,
    ) {
        stats.dropped_unsupported_request_body_ops.push(op_name);
        *operation = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn normalize_operation(
    method: &str,
    path: &str,
    operation: &mut Operation,
    op_name: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    object_schema_refs: &HashSet<String>,
    backend_capabilities: &BackendCapabilities,
    approved_transforms: &CompatibilityTransformPlan,
) -> bool {
    apply_response_variants_transform(
        operation,
        method,
        path,
        op_name,
        warnings,
        approved_transforms,
    );

    let mut dropped_param_indices = Vec::new();
    for (i, param) in operation.parameters.iter_mut().enumerate() {
        if let ReferenceOr::Item(param) = param {
            if !backend_capabilities.supports_optional_object_query_parameters {
                if let Some(param_name) =
                    optional_object_query_param_name(param, object_schema_refs)
                {
                    stats
                        .dropped_optional_object_query_params
                        .push(format!("{op_name}.{param_name}"));
                    dropped_param_indices.push(i);
                    continue;
                }
            }

            let param_data = match param {
                openapiv3::Parameter::Query {
                    parameter_data,
                    style,
                    ..
                } => {
                    if !backend_capabilities.supports_deep_object_query_parameters
                        && *style == QueryStyle::DeepObject
                    {
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
                    backend_capabilities,
                );
            }
        }
    }
    for i in dropped_param_indices.into_iter().rev() {
        operation.parameters.remove(i);
    }

    let request_body_target =
        ContentTarget::OperationRequestBody(OperationTarget::new(method, path));
    if matches!(operation.request_body, Some(ReferenceOr::Reference { .. }))
        && approved_transforms
            .unsupported_request_body_for(&request_body_target)
            .is_some()
    {
        return true;
    }
    if let Some(ReferenceOr::Item(request_body)) = operation.request_body.as_mut() {
        if normalize_request_body(
            request_body,
            op_name,
            warnings,
            stats,
            backend_capabilities,
            approved_transforms,
            &request_body_target,
        ) {
            return true;
        }
        if !backend_capabilities.accepts_schemaless_request_bodies
            && request_body_has_schemaless_content(request_body)
        {
            operation.request_body = None;
            warnings.push(rules::typed_warning(
                typed::SCHEMALESS_REQUEST_BODY_DROPPED,
                format!("normalized {op_name} — dropped requestBody (no schema specified)"),
                Some(ReportSubject::operation(op_name)),
            ));
        }
    }

    for (code, response) in operation.responses.responses.iter_mut() {
        if let ReferenceOr::Item(response) = response {
            normalize_response(
                response,
                op_name,
                warnings,
                stats,
                backend_capabilities,
                approved_transforms,
                &ContentTarget::OperationResponse {
                    operation: OperationTarget::new(method, path),
                    status: code.to_string(),
                },
            );
        }
    }
    if let Some(ReferenceOr::Item(response)) = operation.responses.default.as_mut() {
        normalize_response(
            response,
            op_name,
            warnings,
            stats,
            backend_capabilities,
            approved_transforms,
            &ContentTarget::OperationDefaultResponse(OperationTarget::new(method, path)),
        );
    }
    false
}

fn apply_response_variants_transform(
    operation: &mut Operation,
    method: &str,
    path: &str,
    op_name: &str,
    warnings: &mut Vec<ReportEntry>,
    approved_transforms: &CompatibilityTransformPlan,
) {
    let Some(CompatibilityTransformAction::PruneResponseVariants { kept, dropped, .. }) =
        approved_transforms.response_variants_for(method, path)
    else {
        return;
    };

    operation
        .responses
        .responses
        .retain(|code, _| code.to_string() == *kept);
    if kept != "default" {
        operation.responses.default = None;
    }
    warnings.push(response_variants_report(op_name, kept, dropped));
}

fn normalize_request_body(
    request_body: &mut RequestBody,
    op_name: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    backend_capabilities: &BackendCapabilities,
    approved_transforms: &CompatibilityTransformPlan,
    content_target: &ContentTarget,
) -> bool {
    if let Some(CompatibilityTransformAction::PruneContentTypes { kept, dropped, .. }) =
        approved_transforms.content_for(content_target)
    {
        apply_content_pruning(&mut request_body.content, kept);
        warnings.push(content_types_report(op_name, content_target, kept, dropped));
    }
    if let Some(CompatibilityTransformAction::DropUnsupportedRequestBody { report, .. }) =
        approved_transforms.unsupported_request_body_for(content_target)
    {
        request_body.content.clear();
        if matches!(content_target, ContentTarget::ComponentRequestBody(_)) {
            warnings.push(report.clone());
        }
        return true;
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
                backend_capabilities,
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
    backend_capabilities: &BackendCapabilities,
    approved_transforms: &CompatibilityTransformPlan,
    content_target: &ContentTarget,
) {
    if let Some(CompatibilityTransformAction::PruneContentTypes { kept, dropped, .. }) =
        approved_transforms.content_for(content_target)
    {
        apply_content_pruning(&mut response.content, kept);
        warnings.push(content_types_report(op_name, content_target, kept, dropped));
    }
    for (mime, media_type) in response.content.iter_mut() {
        if let Some(ReferenceOr::Item(schema)) = media_type.schema.as_mut() {
            let _ = normalize_schema(
                schema,
                &format!("{op_name}.response.{mime}"),
                warnings,
                stats,
                backend_capabilities,
            );
        }
    }
}

#[allow(clippy::collapsible_match)]
fn normalize_schema(
    schema: &mut Schema,
    path: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    backend_capabilities: &BackendCapabilities,
) -> Result<()> {
    if !backend_capabilities.supports_schema_defaults && schema.schema_data.default.take().is_some()
    {
        stats.dropped_defaults += 1;
    }

    match &mut schema.schema_kind {
        SchemaKind::Type(Type::String(string)) => {
            if backend_capabilities.requires_unique_sanitized_enum_variants {
                if let Some(colliding) = string_enum_collision(&string.enumeration) {
                    warnings.push(rules::typed_warning(
                    typed::ENUM_CONSTRAINT_DROPPED,
                    format!("normalized {path} — dropped enum constraint (values [{colliding}] collide on Rust identifier sanitization); field is now a free-form string preserving wire format"),
                    Some(ReportSubject::schema(path)),
                ));
                    string.enumeration.clear();
                }
            }
        }
        SchemaKind::Type(Type::Object(object)) => {
            normalize_object_schema(object, path, warnings, stats, backend_capabilities)?
        }
        SchemaKind::Type(Type::Array(array)) => {
            normalize_array_schema(array, path, warnings, stats, backend_capabilities)?
        }
        SchemaKind::OneOf { one_of } => normalize_schema_refs(
            one_of,
            &format!("{path}.oneOf"),
            warnings,
            stats,
            backend_capabilities,
        )?,
        SchemaKind::AllOf { all_of } => normalize_schema_refs(
            all_of,
            &format!("{path}.allOf"),
            warnings,
            stats,
            backend_capabilities,
        )?,
        SchemaKind::AnyOf { any_of } => normalize_schema_refs(
            any_of,
            &format!("{path}.anyOf"),
            warnings,
            stats,
            backend_capabilities,
        )?,
        SchemaKind::Not { not } => normalize_boxed_reference_or_schema(
            not,
            &format!("{path}.not"),
            warnings,
            stats,
            backend_capabilities,
        )?,
        SchemaKind::Any(any) => {
            if let Some(typ) = any.typ.clone() {
                if !is_supported_schema_type(&typ, backend_capabilities) {
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
            if backend_capabilities.requires_unique_sanitized_enum_variants {
                if let Some(colliding) = json_enum_collision(&any.enumeration) {
                    warnings.push(rules::typed_warning(
                    typed::ENUM_CONSTRAINT_DROPPED,
                    format!("normalized {path} — dropped enum constraint (values [{colliding}] collide on Rust identifier sanitization); field is now a free-form string preserving wire format"),
                    Some(ReportSubject::schema(path)),
                ));
                    any.enumeration.clear();
                }
            }
            if backend_capabilities.requires_unique_sanitized_object_properties {
                drop_colliding_properties(&mut any.properties, &mut any.required, path, warnings);
            }
            for (name, property) in any.properties.iter_mut() {
                normalize_boxed_schema_ref(
                    property,
                    &format!("{path}.properties.{name}"),
                    warnings,
                    stats,
                    backend_capabilities,
                )?;
            }
            if let Some(items) = any.items.as_mut() {
                normalize_boxed_schema_ref(
                    items,
                    &format!("{path}.items"),
                    warnings,
                    stats,
                    backend_capabilities,
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
                    backend_capabilities,
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
    backend_capabilities: &BackendCapabilities,
) -> Result<()> {
    if backend_capabilities.requires_unique_sanitized_object_properties {
        drop_colliding_properties(&mut object.properties, &mut object.required, path, warnings);
    }
    for (name, property) in object.properties.iter_mut() {
        normalize_boxed_schema_ref(
            property,
            &format!("{path}.properties.{name}"),
            warnings,
            stats,
            backend_capabilities,
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
            backend_capabilities,
        )?;
    }
    Ok(())
}

fn normalize_array_schema(
    array: &mut ArrayType,
    path: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    backend_capabilities: &BackendCapabilities,
) -> Result<()> {
    if let Some(items) = array.items.as_mut() {
        normalize_boxed_schema_ref(
            items,
            &format!("{path}.items"),
            warnings,
            stats,
            backend_capabilities,
        )?;
    }
    Ok(())
}

fn normalize_schema_refs(
    refs: &mut [ReferenceOr<Schema>],
    path: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    backend_capabilities: &BackendCapabilities,
) -> Result<()> {
    for (i, schema) in refs.iter_mut().enumerate() {
        normalize_schema_ref(
            schema,
            &format!("{path}[{i}]"),
            warnings,
            stats,
            backend_capabilities,
        )?;
    }
    Ok(())
}

fn normalize_schema_ref(
    schema: &mut ReferenceOr<Schema>,
    path: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    backend_capabilities: &BackendCapabilities,
) -> Result<()> {
    if let ReferenceOr::Item(schema) = schema {
        normalize_schema(schema, path, warnings, stats, backend_capabilities)?;
    }
    Ok(())
}

fn normalize_boxed_schema_ref(
    schema: &mut ReferenceOr<Box<Schema>>,
    path: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    backend_capabilities: &BackendCapabilities,
) -> Result<()> {
    if let ReferenceOr::Item(schema) = schema {
        normalize_schema(schema.as_mut(), path, warnings, stats, backend_capabilities)?;
    }
    Ok(())
}

fn normalize_boxed_reference_or_schema(
    schema: &mut Box<ReferenceOr<Schema>>,
    path: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    backend_capabilities: &BackendCapabilities,
) -> Result<()> {
    normalize_schema_ref(schema.as_mut(), path, warnings, stats, backend_capabilities)
}

fn is_supported_schema_type(typ: &str, backend_capabilities: &BackendCapabilities) -> bool {
    backend_capabilities.supported_schema_types.contains(&typ)
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

fn propose_response_content_pruning(
    content: &indexmap::IndexMap<String, MediaType>,
    backend_capabilities: &BackendCapabilities,
) -> Option<(String, Vec<String>)> {
    if !backend_capabilities.requires_single_content_type_per_message || content.len() <= 1 {
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

    Some((kept, dropped))
}

fn propose_request_content_pruning(
    content: &indexmap::IndexMap<String, MediaType>,
    backend_capabilities: &BackendCapabilities,
) -> Option<(String, Vec<String>)> {
    let supported: Vec<String> = content
        .keys()
        .filter(|mime| is_supported_request_mime(mime, backend_capabilities))
        .cloned()
        .collect();
    if supported.is_empty() {
        return None;
    }
    if supported.len() == content.len()
        && (!backend_capabilities.requires_single_content_type_per_message || content.len() <= 1)
    {
        return None;
    }

    let kept = if content.contains_key(JSON_MIME) && supported.iter().any(|mime| mime == JSON_MIME)
    {
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

    Some((kept, dropped))
}

fn apply_content_pruning(content: &mut indexmap::IndexMap<String, MediaType>, kept: &str) {
    let media_type = content
        .get(kept)
        .unwrap_or_else(|| panic!("approved content pruning target {kept} must exist"))
        .clone();
    content.clear();
    content.insert(kept.to_string(), media_type);
}

fn content_has_only_unsupported_request_types(
    content: &indexmap::IndexMap<String, MediaType>,
    backend_capabilities: &BackendCapabilities,
) -> bool {
    !content.is_empty()
        && content
            .keys()
            .all(|mime| !is_supported_request_mime(mime, backend_capabilities))
}

fn is_supported_request_mime(mime: &str, backend_capabilities: &BackendCapabilities) -> bool {
    backend_capabilities
        .supported_request_body_content_types
        .contains(&mime)
}

fn operation_name(method: &str, path: &str, operation: &Operation) -> String {
    traversal::operation_identifier(method, path, operation)
}
