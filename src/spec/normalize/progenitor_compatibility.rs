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
            normalize_schema(
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

pub(super) fn emit_summary_reports(reports: &mut Vec<ReportEntry>, stats: &NormalizeStats) {
    if stats.dropped_defaults > 0 {
        reports.push(schema_defaults_report(stats.dropped_defaults));
    }
    if !stats.dropped_unsupported_request_body_ops.is_empty() {
        reports.push(unsupported_request_body_operations_report(
            &stats.dropped_unsupported_request_body_ops,
        ));
    }
    if !stats.normalized_deep_object_query_params.is_empty() {
        reports.push(deep_object_query_params_report(
            &stats.normalized_deep_object_query_params,
        ));
    }
}

pub(super) fn emit_optional_object_query_param_report(
    reports: &mut Vec<ReportEntry>,
    stats: &NormalizeStats,
) {
    if !stats.dropped_optional_object_query_params.is_empty() {
        reports.push(optional_object_query_params_report(
            &stats.dropped_optional_object_query_params,
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

    fn unsupported_operation_request_body_for(
        &self,
        method: &str,
        path: &str,
    ) -> Option<&OperationRequestBodyDropTarget> {
        self.actions.iter().find_map(|action| {
            if let CompatibilityTransformAction::DropUnsupportedRequestBodyOperations {
                targets,
                ..
            } = action
            {
                targets.iter().find(|target| {
                    target.operation.method == method && target.operation.path == path
                })
            } else {
                None
            }
        })
    }

    fn should_drop_schema_default(&self, path: &str) -> bool {
        self.actions.iter().any(|action| {
            matches!(
                action,
                CompatibilityTransformAction::DropSchemaDefaults { targets, .. }
                    if targets.iter().any(|target| target == path)
            )
        })
    }

    fn deep_object_query_param_for(
        &self,
        operation: &OperationTarget,
        param_name: &str,
    ) -> Option<&ParameterTransformTarget> {
        self.actions.iter().find_map(|action| {
            if let CompatibilityTransformAction::RewriteDeepObjectQueryParams { targets, .. } =
                action
            {
                targets.iter().find(|target| {
                    target.operation == *operation && target.param_name == param_name
                })
            } else {
                None
            }
        })
    }

    fn optional_object_query_param_for(
        &self,
        operation: &OperationTarget,
        param_name: &str,
    ) -> Option<&ParameterTransformTarget> {
        self.actions.iter().find_map(|action| {
            if let CompatibilityTransformAction::DropOptionalObjectQueryParams { targets, .. } =
                action
            {
                targets.iter().find(|target| {
                    target.operation == *operation && target.param_name == param_name
                })
            } else {
                None
            }
        })
    }

    fn schemaless_request_body_for(&self, operation: &OperationTarget) -> Option<&ReportEntry> {
        self.actions.iter().find_map(|action| {
            if let CompatibilityTransformAction::DropSchemalessRequestBody {
                target, report, ..
            } = action
            {
                (target == operation).then_some(report)
            } else {
                None
            }
        })
    }

    fn enum_constraint_for(&self, path: &str) -> Option<&ReportEntry> {
        self.actions.iter().find_map(|action| {
            if let CompatibilityTransformAction::DropEnumConstraint { target, report, .. } = action
            {
                (target == path).then_some(report)
            } else {
                None
            }
        })
    }

    fn unsupported_schema_type_for(&self, path: &str) -> Option<&ReportEntry> {
        self.actions.iter().find_map(|action| {
            if let CompatibilityTransformAction::ReplaceUnsupportedSchemaType {
                target,
                report,
                ..
            } = action
            {
                (target == path).then_some(report)
            } else {
                None
            }
        })
    }

    fn colliding_properties_for(&self, path: &str) -> Vec<&CompatibilityTransformAction> {
        self.actions
            .iter()
            .filter(|action| {
                matches!(
                    action,
                    CompatibilityTransformAction::DropCollidingProperties { target, .. }
                        if target == path
                )
            })
            .collect()
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParameterTransformTarget {
    operation: OperationTarget,
    param_name: String,
    label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OperationRequestBodyDropTarget {
    operation: OperationTarget,
    op_name: String,
}

#[derive(Debug, Clone, Default)]
struct CompatibilityAggregateProposal {
    schema_defaults: Vec<String>,
    unsupported_request_body_ops: Vec<OperationRequestBodyDropTarget>,
    deep_object_query_params: Vec<ParameterTransformTarget>,
    optional_object_query_params: Vec<ParameterTransformTarget>,
}

impl CompatibilityAggregateProposal {
    fn push_actions(self, plan: &mut CompatibilityTransformPlan) {
        if !self.schema_defaults.is_empty() {
            plan.push(CompatibilityTransformAction::DropSchemaDefaults {
                report: schema_defaults_report(self.schema_defaults.len()),
                targets: self.schema_defaults,
            });
        }
        if !self.unsupported_request_body_ops.is_empty() {
            let op_names = self
                .unsupported_request_body_ops
                .iter()
                .map(|target| target.op_name.clone())
                .collect::<Vec<_>>();
            plan.push(
                CompatibilityTransformAction::DropUnsupportedRequestBodyOperations {
                    report: unsupported_request_body_operations_report(&op_names),
                    targets: self.unsupported_request_body_ops,
                },
            );
        }
        if !self.deep_object_query_params.is_empty() {
            let labels = self
                .deep_object_query_params
                .iter()
                .map(|target| target.label.clone())
                .collect::<Vec<_>>();
            plan.push(CompatibilityTransformAction::RewriteDeepObjectQueryParams {
                report: deep_object_query_params_report(&labels),
                targets: self.deep_object_query_params,
            });
        }
        if !self.optional_object_query_params.is_empty() {
            let labels = self
                .optional_object_query_params
                .iter()
                .map(|target| target.label.clone())
                .collect::<Vec<_>>();
            plan.push(
                CompatibilityTransformAction::DropOptionalObjectQueryParams {
                    report: optional_object_query_params_report(&labels),
                    targets: self.optional_object_query_params,
                },
            );
        }
    }
}

#[derive(Debug, Clone)]
enum CompatibilityTransformAction {
    PruneResponseVariants {
        target: OperationTarget,
        kept: String,
        report: ReportEntry,
    },
    PruneContentTypes {
        target: ContentTarget,
        kept: String,
        report: ReportEntry,
    },
    DropUnsupportedRequestBody {
        target: ContentTarget,
        report: ReportEntry,
    },
    DropSchemaDefaults {
        targets: Vec<String>,
        report: ReportEntry,
    },
    DropUnsupportedRequestBodyOperations {
        targets: Vec<OperationRequestBodyDropTarget>,
        report: ReportEntry,
    },
    RewriteDeepObjectQueryParams {
        targets: Vec<ParameterTransformTarget>,
        report: ReportEntry,
    },
    DropOptionalObjectQueryParams {
        targets: Vec<ParameterTransformTarget>,
        report: ReportEntry,
    },
    DropSchemalessRequestBody {
        target: OperationTarget,
        report: ReportEntry,
    },
    DropEnumConstraint {
        target: String,
        report: ReportEntry,
    },
    ReplaceUnsupportedSchemaType {
        target: String,
        report: ReportEntry,
    },
    DropCollidingProperties {
        target: String,
        dropped: Vec<String>,
        report: ReportEntry,
    },
}

impl CompatibilityTransformAction {
    fn report_entry(&self) -> &ReportEntry {
        match self {
            Self::PruneResponseVariants { report, .. }
            | Self::PruneContentTypes { report, .. }
            | Self::DropUnsupportedRequestBody { report, .. }
            | Self::DropSchemaDefaults { report, .. }
            | Self::DropUnsupportedRequestBodyOperations { report, .. }
            | Self::RewriteDeepObjectQueryParams { report, .. }
            | Self::DropOptionalObjectQueryParams { report, .. }
            | Self::DropSchemalessRequestBody { report, .. }
            | Self::DropEnumConstraint { report, .. }
            | Self::ReplaceUnsupportedSchemaType { report, .. }
            | Self::DropCollidingProperties { report, .. } => report,
        }
    }
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
            propose_schema_transforms(
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
                        if !backend_capabilities.accepts_schemaless_request_bodies
                            && request_body_has_schemaless_content(request_body)
                        {
                            plan.push(CompatibilityTransformAction::DropSchemalessRequestBody {
                                target: target.clone(),
                                report: schemaless_request_body_report(&op_name),
                            });
                        }
                    }
                }
                ReferenceOr::Reference { reference } => {
                    if let Some(content) = component_request_body_content(spec, reference) {
                        if content_has_only_unsupported_request_types(content, backend_capabilities)
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
            if !backend_capabilities.supports_optional_object_query_parameters {
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
                    if !backend_capabilities.supports_deep_object_query_parameters
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
                propose_schema_transforms(
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
            propose_schema_transforms(
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
    let (kept, dropped) = propose_response_variant_pruning(operation, backend_capabilities)?;
    plan.push(CompatibilityTransformAction::PruneResponseVariants {
        target,
        report: response_variants_report(op_name, &kept, &dropped),
        kept: kept.clone(),
    });
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
    if content_has_only_unsupported_request_types(content, backend_capabilities) {
        if let ContentTarget::OperationRequestBody(operation) = &target {
            aggregate
                .unsupported_request_body_ops
                .push(OperationRequestBodyDropTarget {
                    operation: operation.clone(),
                    op_name: op_name.to_string(),
                });
        } else {
            plan.push(CompatibilityTransformAction::DropUnsupportedRequestBody {
                report: unsupported_request_body_report(op_name, &target, content),
                target,
            });
        }
        return ProposedContentTransform {
            drops_body: true,
            kept: None,
        };
    }

    if let Some((kept, dropped)) = propose_request_content_pruning(content, backend_capabilities) {
        plan.push(CompatibilityTransformAction::PruneContentTypes {
            report: content_types_report(op_name, &target, &kept, &dropped),
            target,
            kept: kept.clone(),
        });
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
    if let Some((kept, dropped)) = propose_response_content_pruning(content, backend_capabilities) {
        plan.push(CompatibilityTransformAction::PruneContentTypes {
            report: content_types_report(op_name, &target, &kept, &dropped),
            target,
            kept: kept.clone(),
        });
        return Some(kept);
    }
    None
}

#[derive(Debug, Clone, Default)]
struct ProposedContentTransform {
    drops_body: bool,
    kept: Option<String>,
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

fn schema_defaults_report(count: usize) -> ReportEntry {
    rules::typed_warning(
        typed::SCHEMA_DEFAULTS_DROPPED,
        format!("normalized {count} schemas — dropped default values"),
        None,
    )
}

fn unsupported_request_body_operations_report(op_names: &[String]) -> ReportEntry {
    rules::typed_warning(
        typed::UNSUPPORTED_REQUEST_BODIES_DROPPED,
        format!(
            "dropped {} operations with progenitor-unsupported request body: {}",
            op_names.len(),
            op_names.join(", ")
        ),
        None,
    )
}

fn deep_object_query_params_report(labels: &[String]) -> ReportEntry {
    rules::typed_warning(
        typed::DEEP_OBJECT_QUERY_PARAMS_REWRITTEN,
        format!(
            "normalized {} query parameters — replaced unsupported deepObject style with form: {}",
            labels.len(),
            labels.join(", ")
        ),
        None,
    )
}

fn optional_object_query_params_report(labels: &[String]) -> ReportEntry {
    rules::typed_warning(
        typed::OPTIONAL_OBJECT_QUERY_PARAMS_DROPPED,
        format!(
            "dropped {} optional object query parameters with progenitor-unsupported builder shape: {}",
            labels.len(),
            labels.join(", ")
        ),
        None,
    )
}

fn schemaless_request_body_report(op_name: &str) -> ReportEntry {
    rules::typed_warning(
        typed::SCHEMALESS_REQUEST_BODY_DROPPED,
        format!("normalized {op_name} — dropped requestBody (no schema specified)"),
        Some(ReportSubject::operation(op_name)),
    )
}

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

#[allow(clippy::collapsible_match)]
fn propose_schema_transforms(
    schema: &Schema,
    path: &str,
    plan: &mut CompatibilityTransformPlan,
    aggregate: &mut CompatibilityAggregateProposal,
    backend_capabilities: &BackendCapabilities,
) {
    if !backend_capabilities.supports_schema_defaults && schema.schema_data.default.is_some() {
        aggregate.schema_defaults.push(path.to_string());
    }

    match &schema.schema_kind {
        SchemaKind::Type(Type::String(string)) => {
            if backend_capabilities.requires_unique_sanitized_enum_variants {
                if let Some(colliding) = string_enum_collision(&string.enumeration) {
                    plan.push(CompatibilityTransformAction::DropEnumConstraint {
                        target: path.to_string(),
                        report: enum_constraint_report(path, &colliding),
                    });
                }
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
            if backend_capabilities.requires_unique_sanitized_enum_variants {
                if let Some(colliding) = json_enum_collision(&any.enumeration) {
                    plan.push(CompatibilityTransformAction::DropEnumConstraint {
                        target: path.to_string(),
                        report: enum_constraint_report(path, &colliding),
                    });
                }
            }
            let dropped = if backend_capabilities.requires_unique_sanitized_object_properties {
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
    let dropped = if backend_capabilities.requires_unique_sanitized_object_properties {
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
                let _ = normalize_schema(
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
        if request_body_has_schemaless_content(request_body) {
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
    let Some(CompatibilityTransformAction::PruneResponseVariants { kept, report, .. }) =
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
    warnings.push(report.clone());
}

fn normalize_request_body(
    request_body: &mut RequestBody,
    op_name: &str,
    warnings: &mut Vec<ReportEntry>,
    stats: &mut NormalizeStats,
    approved_transforms: &CompatibilityTransformPlan,
    content_target: &ContentTarget,
) -> bool {
    if let Some(CompatibilityTransformAction::PruneContentTypes { kept, report, .. }) =
        approved_transforms.content_for(content_target)
    {
        apply_content_pruning(&mut request_body.content, kept);
        warnings.push(report.clone());
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
        return false;
    }
    for (mime, media_type) in request_body.content.iter_mut() {
        if let Some(ReferenceOr::Item(schema)) = media_type.schema.as_mut() {
            let _ = normalize_schema(
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
    if let Some(CompatibilityTransformAction::PruneContentTypes { kept, report, .. }) =
        approved_transforms.content_for(content_target)
    {
        apply_content_pruning(&mut response.content, kept);
        warnings.push(report.clone());
    }
    for (mime, media_type) in response.content.iter_mut() {
        if let Some(ReferenceOr::Item(schema)) = media_type.schema.as_mut() {
            let _ = normalize_schema(
                schema,
                &format!("{op_name}.response.{mime}"),
                warnings,
                stats,
                approved_transforms,
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
    approved_transforms: &CompatibilityTransformPlan,
) -> Result<()> {
    if approved_transforms.should_drop_schema_default(path)
        && schema.schema_data.default.take().is_some()
    {
        stats.dropped_defaults += 1;
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
    backend_capabilities.supported_schema_types.contains(&typ)
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
