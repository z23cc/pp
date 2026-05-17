use anyhow::Result;
use openapiv3::{MediaType, OpenAPI, Operation, QueryStyle, ReferenceOr, RequestBody, Response};
use std::collections::HashSet;

use crate::backend::BackendCapabilities;
use crate::spec::report::{ReportEntry, ReportSubject};
use crate::spec::traversal;

pub(super) const JSON_MIME: &str = "application/json";
#[cfg(test)]
pub(super) const FORM_MIME: &str = "application/x-www-form-urlencoded";

mod actions;
mod content_types;
mod query_params;
mod request_bodies;
mod response_variants;
mod schema_defaults;
mod schemas;

pub(crate) use actions::CompatibilityTransformPlan;
use actions::{
    CompatibilityAggregateProposal, CompatibilityTransformAction, ContentTarget,
    OperationRequestBodyDropTarget, OperationTarget, ParameterTransformTarget,
};

pub(crate) fn propose_transforms(
    spec: &OpenAPI,
    backend_capabilities: &BackendCapabilities,
) -> CompatibilityTransformPlan {
    let mut plan = CompatibilityTransformPlan::default();
    let mut aggregate = CompatibilityAggregateProposal::default();
    let object_schema_refs = component_object_schema_refs(spec);
    propose_component_transforms(spec, &mut plan, &mut aggregate, backend_capabilities);
    propose_operation_transforms(
        spec,
        &mut plan,
        &mut aggregate,
        &object_schema_refs,
        backend_capabilities,
    );
    aggregate.push_actions(&mut plan);
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
    approved_transforms.emit_applied_aggregate_reports(reports, &stats)?;
    Ok(stats)
}

fn normalize_components(
    spec: &mut OpenAPI,
    reports: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    _backend_capabilities: &BackendCapabilities,
    approved_transforms: &CompatibilityTransformPlan,
) -> Result<()> {
    let Some(components) = spec.components.as_mut() else {
        return Ok(());
    };
    for (name, schema) in components.schemas.iter_mut() {
        if let ReferenceOr::Item(schema) = schema {
            schemas::normalize_schema(
                schema,
                &format!("component schema {name}"),
                reports,
                stats,
                approved_transforms,
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

#[derive(Default)]
pub(super) struct NormalizeStats {
    dropped_schema_defaults: Vec<String>,
    dropped_unsupported_request_body_ops: Vec<String>,
    normalized_deep_object_query_params: Vec<String>,
    dropped_optional_object_query_params: Vec<String>,
}

fn propose_component_transforms(
    spec: &OpenAPI,
    plan: &mut CompatibilityTransformPlan,
    aggregate: &mut CompatibilityAggregateProposal,
    backend_capabilities: &BackendCapabilities,
) {
    let Some(components) = spec.components.as_ref() else {
        return;
    };
    for (name, schema) in &components.schemas {
        if let ReferenceOr::Item(schema) = schema {
            schemas::propose_schema_transforms(
                schema,
                &format!("component schema {name}"),
                plan,
                aggregate,
                backend_capabilities,
            );
        }
    }
    for (name, request_body) in &components.request_bodies {
        if let ReferenceOr::Item(request_body) = request_body {
            let outcome = propose_request_content_transform(
                &request_body.content,
                &format!("component requestBody {name}"),
                ContentTarget::ComponentRequestBody(name.clone()),
                plan,
                aggregate,
                backend_capabilities,
            );
            if !outcome.drops_body {
                propose_content_schema_transforms(
                    &request_body.content,
                    outcome.kept.as_deref(),
                    &format!("component requestBody {name}"),
                    "requestBody",
                    plan,
                    aggregate,
                    backend_capabilities,
                );
            }
        }
    }
    for (name, response) in &components.responses {
        if let ReferenceOr::Item(response) = response {
            let kept = propose_response_content_transform(
                &response.content,
                &format!("component response {name}"),
                ContentTarget::ComponentResponse(name.clone()),
                plan,
                backend_capabilities,
            );
            propose_content_schema_transforms(
                &response.content,
                kept.as_deref(),
                &format!("component response {name}"),
                "response",
                plan,
                aggregate,
                backend_capabilities,
            );
        }
    }
}

fn propose_operation_transforms(
    spec: &OpenAPI,
    plan: &mut CompatibilityTransformPlan,
    aggregate: &mut CompatibilityAggregateProposal,
    object_schema_refs: &HashSet<String>,
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

        propose_parameter_transforms(
            operation_ref.operation,
            &op_name,
            &target,
            object_schema_refs,
            plan,
            aggregate,
            backend_capabilities,
        );

        let mut operation_dropped_for_request_body = false;
        if let Some(request_body) = operation_ref.operation.request_body.as_ref() {
            match request_body {
                ReferenceOr::Item(request_body) => {
                    let outcome = propose_request_content_transform(
                        &request_body.content,
                        &op_name,
                        ContentTarget::OperationRequestBody(target.clone()),
                        plan,
                        aggregate,
                        backend_capabilities,
                    );
                    if outcome.drops_body {
                        operation_dropped_for_request_body = true;
                    } else {
                        propose_content_schema_transforms(
                            &request_body.content,
                            outcome.kept.as_deref(),
                            &op_name,
                            "requestBody",
                            plan,
                            aggregate,
                            backend_capabilities,
                        );
                        if !backend_capabilities.request_bodies.accepts_schemaless
                            && request_bodies::has_schemaless_content(request_body)
                        {
                            plan.push(CompatibilityTransformAction::DropSchemalessRequestBody(
                                request_bodies::SchemalessAction::new(target.clone(), &op_name),
                            ));
                        }
                    }
                }
                ReferenceOr::Reference { reference } => {
                    if let Some(content) = component_request_body_content(spec, reference) {
                        if request_bodies::has_only_unsupported_types(content, backend_capabilities)
                        {
                            aggregate.unsupported_request_body_ops.push(
                                OperationRequestBodyDropTarget {
                                    operation: target.clone(),
                                    op_name: op_name.clone(),
                                },
                            );
                            operation_dropped_for_request_body = true;
                        }
                    }
                }
            }
        }

        if operation_dropped_for_request_body {
            continue;
        }

        for (code, response) in &operation_ref.operation.responses.responses {
            let status = code.to_string();
            if kept_response.as_ref().is_some_and(|kept| kept != &status) {
                continue;
            }
            if let ReferenceOr::Item(response) = response {
                let kept = propose_response_content_transform(
                    &response.content,
                    &op_name,
                    ContentTarget::OperationResponse {
                        operation: target.clone(),
                        status,
                    },
                    plan,
                    backend_capabilities,
                );
                propose_content_schema_transforms(
                    &response.content,
                    kept.as_deref(),
                    &op_name,
                    "response",
                    plan,
                    aggregate,
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
                let kept = propose_response_content_transform(
                    &response.content,
                    &op_name,
                    ContentTarget::OperationDefaultResponse(target),
                    plan,
                    backend_capabilities,
                );
                propose_content_schema_transforms(
                    &response.content,
                    kept.as_deref(),
                    &op_name,
                    "response",
                    plan,
                    aggregate,
                    backend_capabilities,
                );
            }
        }
    }
}

fn propose_parameter_transforms(
    operation: &Operation,
    op_name: &str,
    target: &OperationTarget,
    object_schema_refs: &HashSet<String>,
    plan: &mut CompatibilityTransformPlan,
    aggregate: &mut CompatibilityAggregateProposal,
    backend_capabilities: &BackendCapabilities,
) {
    for (i, param) in operation.parameters.iter().enumerate() {
        if let ReferenceOr::Item(param) = param {
            if !backend_capabilities
                .parameters
                .supports_optional_object_query
            {
                if let Some(param_name) =
                    optional_object_query_param_name(param, object_schema_refs)
                {
                    aggregate
                        .optional_object_query_params
                        .push(ParameterTransformTarget {
                            operation: target.clone(),
                            label: format!("{op_name}.{param_name}"),
                            param_name,
                        });
                    continue;
                }
            }

            let param_data = match param {
                openapiv3::Parameter::Query {
                    parameter_data,
                    style,
                    ..
                } => {
                    if !backend_capabilities.parameters.supports_deep_object_query
                        && *style == QueryStyle::DeepObject
                    {
                        aggregate
                            .deep_object_query_params
                            .push(ParameterTransformTarget {
                                operation: target.clone(),
                                label: format!("{op_name}.{}", parameter_data.name),
                                param_name: parameter_data.name.clone(),
                            });
                    }
                    parameter_data
                }
                openapiv3::Parameter::Header { parameter_data, .. }
                | openapiv3::Parameter::Path { parameter_data, .. }
                | openapiv3::Parameter::Cookie { parameter_data, .. } => parameter_data,
            };
            if let openapiv3::ParameterSchemaOrContent::Schema(ReferenceOr::Item(schema)) =
                &param_data.format
            {
                schemas::propose_schema_transforms(
                    schema,
                    &format!("{op_name}.parameters[{i}].{}", param_data.name),
                    plan,
                    aggregate,
                    backend_capabilities,
                );
            }
        }
    }
}

fn propose_content_schema_transforms(
    content: &indexmap::IndexMap<String, MediaType>,
    kept: Option<&str>,
    op_name: &str,
    segment: &str,
    plan: &mut CompatibilityTransformPlan,
    aggregate: &mut CompatibilityAggregateProposal,
    backend_capabilities: &BackendCapabilities,
) {
    for (mime, media_type) in content {
        if kept.is_some_and(|kept| kept != mime) {
            continue;
        }
        if let Some(ReferenceOr::Item(schema)) = media_type.schema.as_ref() {
            schemas::propose_schema_transforms(
                schema,
                &format!("{op_name}.{segment}.{mime}"),
                plan,
                aggregate,
                backend_capabilities,
            );
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
    let action = response_variants::propose(operation, op_name, target, backend_capabilities)?;
    let kept = action.kept().to_string();
    plan.push(CompatibilityTransformAction::PruneResponseVariants(action));
    Some(kept)
}

fn propose_request_content_transform(
    content: &indexmap::IndexMap<String, MediaType>,
    op_name: &str,
    target: ContentTarget,
    plan: &mut CompatibilityTransformPlan,
    aggregate: &mut CompatibilityAggregateProposal,
    backend_capabilities: &BackendCapabilities,
) -> ProposedContentTransform {
    if request_bodies::has_only_unsupported_types(content, backend_capabilities) {
        if let ContentTarget::OperationRequestBody(operation) = &target {
            aggregate
                .unsupported_request_body_ops
                .push(OperationRequestBodyDropTarget {
                    operation: operation.clone(),
                    op_name: op_name.to_string(),
                });
        } else {
            plan.push(CompatibilityTransformAction::DropUnsupportedRequestBody(
                request_bodies::UnsupportedAction::new(op_name, target, content),
            ));
        }
        return ProposedContentTransform {
            drops_body: true,
            kept: None,
        };
    }

    if let Some(action) =
        content_types::propose_request(content, op_name, target, backend_capabilities)
    {
        let kept = action.kept().to_string();
        plan.push(CompatibilityTransformAction::PruneContentTypes(action));
        return ProposedContentTransform {
            drops_body: false,
            kept: Some(kept),
        };
    }
    ProposedContentTransform::default()
}

fn propose_response_content_transform(
    content: &indexmap::IndexMap<String, MediaType>,
    op_name: &str,
    target: ContentTarget,
    plan: &mut CompatibilityTransformPlan,
    backend_capabilities: &BackendCapabilities,
) -> Option<String> {
    if let Some(action) =
        content_types::propose_response(content, op_name, target, backend_capabilities)
    {
        let kept = action.kept().to_string();
        plan.push(CompatibilityTransformAction::PruneContentTypes(action));
        return Some(kept);
    }
    None
}

#[derive(Debug, Clone, Default)]
struct ProposedContentTransform {
    drops_body: bool,
    kept: Option<String>,
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
            if schemas::schema_is_object_shaped(schema) {
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
        ReferenceOr::Item(schema) => schemas::schema_is_object_shaped(schema),
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
    _backend_capabilities: &BackendCapabilities,
    approved_transforms: &CompatibilityTransformPlan,
) -> bool {
    apply_response_variants_transform(operation, method, path, warnings, approved_transforms);

    let mut dropped_param_indices = Vec::new();
    let operation_target = OperationTarget::new(method, path);
    for (i, param) in operation.parameters.iter_mut().enumerate() {
        if let ReferenceOr::Item(param) = param {
            if let Some(param_name) = optional_object_query_param_name(param, object_schema_refs) {
                if let Some(target) = approved_transforms
                    .optional_object_query_param_for(&operation_target, &param_name)
                {
                    stats
                        .dropped_optional_object_query_params
                        .push(target.label.clone());
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
                    if *style == QueryStyle::DeepObject {
                        if let Some(target) = approved_transforms
                            .deep_object_query_param_for(&operation_target, &parameter_data.name)
                        {
                            *style = QueryStyle::Form;
                            stats
                                .normalized_deep_object_query_params
                                .push(target.label.clone());
                        }
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
                let _ = schemas::normalize_schema(
                    schema,
                    &format!("{op_name}.parameters[{i}].{}", param_data.name),
                    warnings,
                    stats,
                    approved_transforms,
                );
            }
        }
    }
    for i in dropped_param_indices.into_iter().rev() {
        operation.parameters.remove(i);
    }

    let request_body_target =
        ContentTarget::OperationRequestBody(OperationTarget::new(method, path));
    if operation.request_body.is_some() {
        if let Some(target) =
            approved_transforms.unsupported_operation_request_body_for(method, path)
        {
            stats
                .dropped_unsupported_request_body_ops
                .push(target.op_name.clone());
            return true;
        }
    }
    if let Some(ReferenceOr::Item(request_body)) = operation.request_body.as_mut() {
        if normalize_request_body(
            request_body,
            op_name,
            warnings,
            stats,
            approved_transforms,
            &request_body_target,
        ) {
            stats
                .dropped_unsupported_request_body_ops
                .push(op_name.to_string());
            return true;
        }
        if request_bodies::has_schemaless_content(request_body) {
            if let Some(report) = approved_transforms.schemaless_request_body_for(&operation_target)
            {
                operation.request_body = None;
                warnings.push(report.clone());
            }
        }
    }

    for (code, response) in operation.responses.responses.iter_mut() {
        if let ReferenceOr::Item(response) = response {
            normalize_response(
                response,
                op_name,
                warnings,
                stats,
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
    warnings: &mut Vec<ReportEntry>,
    approved_transforms: &CompatibilityTransformPlan,
) {
    let Some(action) = approved_transforms.response_variants_for(method, path) else {
        return;
    };

    action.apply_approved(operation, warnings);
}

fn normalize_request_body(
    request_body: &mut RequestBody,
    op_name: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    approved_transforms: &CompatibilityTransformPlan,
    content_target: &ContentTarget,
) -> bool {
    if let Some(CompatibilityTransformAction::PruneContentTypes(action)) =
        approved_transforms.content_for(content_target)
    {
        action.apply_approved(&mut request_body.content, warnings);
    }
    if let Some(CompatibilityTransformAction::DropUnsupportedRequestBody(action)) =
        approved_transforms.unsupported_request_body_for(content_target)
    {
        return action.apply_approved(request_body, content_target, warnings);
    }
    if request_body.content.is_empty() {
        return false;
    }
    for (mime, media_type) in request_body.content.iter_mut() {
        if let Some(ReferenceOr::Item(schema)) = media_type.schema.as_mut() {
            let _ = schemas::normalize_schema(
                schema,
                &format!("{op_name}.requestBody.{mime}"),
                warnings,
                stats,
                approved_transforms,
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
    approved_transforms: &CompatibilityTransformPlan,
    content_target: &ContentTarget,
) {
    if let Some(CompatibilityTransformAction::PruneContentTypes(action)) =
        approved_transforms.content_for(content_target)
    {
        action.apply_approved(&mut response.content, warnings);
    }
    for (mime, media_type) in response.content.iter_mut() {
        if let Some(ReferenceOr::Item(schema)) = media_type.schema.as_mut() {
            let _ = schemas::normalize_schema(
                schema,
                &format!("{op_name}.response.{mime}"),
                warnings,
                stats,
                approved_transforms,
            );
        }
    }
}

fn operation_name(method: &str, path: &str, operation: &Operation) -> String {
    traversal::operation_identifier(method, path, operation)
}
