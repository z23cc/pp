use anyhow::{anyhow, Result};
use serde_json::json;

use super::{
    content_types, query_params, request_bodies, response_variants, schema_defaults, NormalizeStats,
};
use crate::spec::normalization_rules::typed;
use crate::spec::report::ReportEntry;
use crate::spec::transform::{json_pointer_escape, TransformActionKind, TransformAuditEntry};

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

    pub(crate) fn audit_entries(&self) -> Vec<TransformAuditEntry> {
        self.actions
            .iter()
            .flat_map(CompatibilityTransformAction::audit_entries)
            .collect()
    }

    pub(in crate::spec::normalize::progenitor_compatibility) fn push(
        &mut self,
        action: CompatibilityTransformAction,
    ) {
        self.actions.push(action);
    }

    pub(in crate::spec::normalize::progenitor_compatibility) fn emit_applied_aggregate_reports(
        &self,
        reports: &mut Vec<ReportEntry>,
        stats: &NormalizeStats,
    ) -> Result<()> {
        let mut saw_schema_defaults = false;
        let mut saw_unsupported_request_bodies = false;
        let mut saw_deep_object_query_params = false;
        let mut saw_optional_object_query_params = false;

        for action in &self.actions {
            match action {
                CompatibilityTransformAction::DropSchemaDefaults(action) => {
                    saw_schema_defaults = true;
                    ensure_aggregate_labels(
                        action.report_entry().code,
                        action.targets(),
                        &stats.dropped_schema_defaults,
                    )?;
                    reports.push(action.report_entry().clone());
                }
                CompatibilityTransformAction::DropUnsupportedRequestBodyOperations(action) => {
                    saw_unsupported_request_bodies = true;
                    let expected = action
                        .targets()
                        .iter()
                        .map(|target| target.op_name.clone())
                        .collect::<Vec<_>>();
                    ensure_aggregate_labels(
                        action.report_entry().code,
                        &expected,
                        &stats.dropped_unsupported_request_body_ops,
                    )?;
                    reports.push(action.report_entry().clone());
                }
                CompatibilityTransformAction::RewriteDeepObjectQueryParams(action) => {
                    saw_deep_object_query_params = true;
                    ensure_aggregate_labels(
                        action.report_entry().code,
                        &action.labels(),
                        &stats.normalized_deep_object_query_params,
                    )?;
                    reports.push(action.report_entry().clone());
                }
                CompatibilityTransformAction::DropOptionalObjectQueryParams(action) => {
                    saw_optional_object_query_params = true;
                    ensure_aggregate_labels(
                        action.report_entry().code,
                        &action.labels(),
                        &stats.dropped_optional_object_query_params,
                    )?;
                    reports.push(action.report_entry().clone());
                }
                _ => {}
            }
        }

        if !saw_schema_defaults && !stats.dropped_schema_defaults.is_empty() {
            return Err(aggregate_drift_error(
                typed::SCHEMA_DEFAULTS_DROPPED,
                "no approved aggregate action",
                &format!("applied {:?}", stats.dropped_schema_defaults),
            ));
        }
        if !saw_unsupported_request_bodies && !stats.dropped_unsupported_request_body_ops.is_empty()
        {
            return Err(aggregate_drift_error(
                typed::UNSUPPORTED_REQUEST_BODIES_DROPPED,
                "no approved aggregate action",
                &format!("applied {:?}", stats.dropped_unsupported_request_body_ops),
            ));
        }
        if !saw_deep_object_query_params && !stats.normalized_deep_object_query_params.is_empty() {
            return Err(aggregate_drift_error(
                typed::DEEP_OBJECT_QUERY_PARAMS_REWRITTEN,
                "no approved aggregate action",
                &format!("applied {:?}", stats.normalized_deep_object_query_params),
            ));
        }
        if !saw_optional_object_query_params
            && !stats.dropped_optional_object_query_params.is_empty()
        {
            return Err(aggregate_drift_error(
                typed::OPTIONAL_OBJECT_QUERY_PARAMS_DROPPED,
                "no approved aggregate action",
                &format!("applied {:?}", stats.dropped_optional_object_query_params),
            ));
        }

        Ok(())
    }

    pub(in crate::spec::normalize::progenitor_compatibility) fn response_variants_for(
        &self,
        method: &str,
        path: &str,
    ) -> Option<&response_variants::Action> {
        self.actions.iter().find_map(|action| {
            if let CompatibilityTransformAction::PruneResponseVariants(action) = action {
                action.matches(method, path).then_some(action)
            } else {
                None
            }
        })
    }

    pub(in crate::spec::normalize::progenitor_compatibility) fn content_for(
        &self,
        target: &ContentTarget,
    ) -> Option<&CompatibilityTransformAction> {
        self.actions.iter().find(|action| {
            matches!(
                action,
                CompatibilityTransformAction::PruneContentTypes(action)
                    if action.target() == target
            )
        })
    }

    pub(in crate::spec::normalize::progenitor_compatibility) fn unsupported_request_body_for(
        &self,
        target: &ContentTarget,
    ) -> Option<&CompatibilityTransformAction> {
        self.actions.iter().find(|action| {
            matches!(
                action,
                CompatibilityTransformAction::DropUnsupportedRequestBody(action)
                    if action.target() == target
            )
        })
    }

    pub(in crate::spec::normalize::progenitor_compatibility) fn unsupported_operation_request_body_for(
        &self,
        method: &str,
        path: &str,
    ) -> Option<&OperationRequestBodyDropTarget> {
        self.actions.iter().find_map(|action| {
            if let CompatibilityTransformAction::DropUnsupportedRequestBodyOperations(action) =
                action
            {
                action.target_for(method, path)
            } else {
                None
            }
        })
    }

    pub(in crate::spec::normalize::progenitor_compatibility) fn should_drop_schema_default(
        &self,
        path: &str,
    ) -> bool {
        self.actions.iter().any(|action| {
            matches!(
                action,
                CompatibilityTransformAction::DropSchemaDefaults(action)
                    if action.contains(path)
            )
        })
    }

    pub(in crate::spec::normalize::progenitor_compatibility) fn deep_object_query_param_for(
        &self,
        operation: &OperationTarget,
        param_name: &str,
    ) -> Option<&ParameterTransformTarget> {
        self.actions.iter().find_map(|action| {
            if let CompatibilityTransformAction::RewriteDeepObjectQueryParams(action) = action {
                action.targets().iter().find(|target| {
                    target.operation == *operation && target.param_name == param_name
                })
            } else {
                None
            }
        })
    }

    pub(in crate::spec::normalize::progenitor_compatibility) fn optional_object_query_param_for(
        &self,
        operation: &OperationTarget,
        param_name: &str,
    ) -> Option<&ParameterTransformTarget> {
        self.actions.iter().find_map(|action| {
            if let CompatibilityTransformAction::DropOptionalObjectQueryParams(action) = action {
                action.targets().iter().find(|target| {
                    target.operation == *operation && target.param_name == param_name
                })
            } else {
                None
            }
        })
    }

    pub(in crate::spec::normalize::progenitor_compatibility) fn schemaless_request_body_for(
        &self,
        operation: &OperationTarget,
    ) -> Option<&ReportEntry> {
        self.actions.iter().find_map(|action| {
            if let CompatibilityTransformAction::DropSchemalessRequestBody(action) = action {
                action.matches(operation).then_some(action.report_entry())
            } else {
                None
            }
        })
    }

    pub(in crate::spec::normalize::progenitor_compatibility) fn enum_constraint_for(
        &self,
        path: &str,
    ) -> Option<&ReportEntry> {
        self.actions.iter().find_map(|action| {
            if let CompatibilityTransformAction::DropEnumConstraint { target, report, .. } = action
            {
                (target == path).then_some(report)
            } else {
                None
            }
        })
    }

    pub(in crate::spec::normalize::progenitor_compatibility) fn unsupported_schema_type_for(
        &self,
        path: &str,
    ) -> Option<&ReportEntry> {
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

    pub(in crate::spec::normalize::progenitor_compatibility) fn colliding_properties_for(
        &self,
        path: &str,
    ) -> Vec<&CompatibilityTransformAction> {
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

fn ensure_aggregate_labels(
    code: &'static str,
    expected: &[String],
    actual: &[String],
) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(aggregate_drift_error(
            code,
            &format!("planned {expected:?}"),
            &format!("applied {actual:?}"),
        ))
    }
}

fn aggregate_drift_error(code: &'static str, expected: &str, actual: &str) -> anyhow::Error {
    anyhow!("approved aggregate report drift for {code}: {expected}; {actual}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::spec::normalize::progenitor_compatibility) struct OperationTarget {
    pub(in crate::spec::normalize::progenitor_compatibility) method: String,
    pub(in crate::spec::normalize::progenitor_compatibility) path: String,
}

impl OperationTarget {
    pub(in crate::spec::normalize::progenitor_compatibility) fn new(
        method: &str,
        path: &str,
    ) -> Self {
        Self {
            method: method.to_string(),
            path: path.to_string(),
        }
    }

    pub(in crate::spec::normalize::progenitor_compatibility) fn label(&self) -> String {
        format!("{} {}", self.method, self.path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::spec::normalize::progenitor_compatibility) enum ContentTarget {
    ComponentRequestBody(String),
    ComponentResponse(String),
    OperationRequestBody(OperationTarget),
    OperationResponse {
        operation: OperationTarget,
        status: String,
    },
    OperationDefaultResponse(OperationTarget),
}

pub(in crate::spec::normalize::progenitor_compatibility) type ParameterTransformTarget =
    query_params::Target;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::spec::normalize::progenitor_compatibility) struct OperationRequestBodyDropTarget {
    pub(in crate::spec::normalize::progenitor_compatibility) operation: OperationTarget,
    pub(in crate::spec::normalize::progenitor_compatibility) op_name: String,
}

#[derive(Debug, Clone, Default)]
pub(in crate::spec::normalize::progenitor_compatibility) struct CompatibilityAggregateProposal {
    pub(in crate::spec::normalize::progenitor_compatibility) schema_defaults: Vec<String>,
    pub(in crate::spec::normalize::progenitor_compatibility) unsupported_request_body_ops:
        Vec<OperationRequestBodyDropTarget>,
    pub(in crate::spec::normalize::progenitor_compatibility) deep_object_query_params:
        Vec<ParameterTransformTarget>,
    pub(in crate::spec::normalize::progenitor_compatibility) optional_object_query_params:
        Vec<ParameterTransformTarget>,
}

impl CompatibilityAggregateProposal {
    pub(in crate::spec::normalize::progenitor_compatibility) fn push_actions(
        self,
        plan: &mut CompatibilityTransformPlan,
    ) {
        if !self.schema_defaults.is_empty() {
            plan.push(CompatibilityTransformAction::DropSchemaDefaults(
                schema_defaults::Action::new(self.schema_defaults),
            ));
        }
        if !self.unsupported_request_body_ops.is_empty() {
            plan.push(
                CompatibilityTransformAction::DropUnsupportedRequestBodyOperations(
                    request_bodies::UnsupportedOperationsAction::new(
                        self.unsupported_request_body_ops,
                    ),
                ),
            );
        }
        if !self.deep_object_query_params.is_empty() {
            plan.push(CompatibilityTransformAction::RewriteDeepObjectQueryParams(
                query_params::DeepObjectAction::new(self.deep_object_query_params),
            ));
        }
        if !self.optional_object_query_params.is_empty() {
            plan.push(CompatibilityTransformAction::DropOptionalObjectQueryParams(
                query_params::OptionalObjectAction::new(self.optional_object_query_params),
            ));
        }
    }
}

#[derive(Debug, Clone)]
pub(in crate::spec::normalize::progenitor_compatibility) enum CompatibilityTransformAction {
    PruneResponseVariants(response_variants::Action),
    PruneContentTypes(content_types::Action),
    DropUnsupportedRequestBody(request_bodies::UnsupportedAction),
    DropSchemaDefaults(schema_defaults::Action),
    DropUnsupportedRequestBodyOperations(request_bodies::UnsupportedOperationsAction),
    RewriteDeepObjectQueryParams(query_params::DeepObjectAction),
    DropOptionalObjectQueryParams(query_params::OptionalObjectAction),
    DropSchemalessRequestBody(request_bodies::SchemalessAction),
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
    pub(in crate::spec::normalize::progenitor_compatibility) fn report_entry(
        &self,
    ) -> &ReportEntry {
        match self {
            Self::DropSchemaDefaults(action) => action.report_entry(),
            Self::PruneResponseVariants(action) => action.report_entry(),
            Self::PruneContentTypes(action) => action.report_entry(),
            Self::DropUnsupportedRequestBody(action) => action.report_entry(),
            Self::DropUnsupportedRequestBodyOperations(action) => action.report_entry(),
            Self::DropSchemalessRequestBody(action) => action.report_entry(),
            Self::DropEnumConstraint { report, .. }
            | Self::ReplaceUnsupportedSchemaType { report, .. }
            | Self::DropCollidingProperties { report, .. } => report,
            Self::RewriteDeepObjectQueryParams(action) => action.report_entry(),
            Self::DropOptionalObjectQueryParams(action) => action.report_entry(),
        }
    }

    fn audit_entries(&self) -> Vec<TransformAuditEntry> {
        let report = self.report_entry();
        match self {
            Self::PruneResponseVariants(action) => action.audit_entries(),
            Self::PruneContentTypes(action) => action.audit_entries(),
            Self::DropUnsupportedRequestBody(action) => action.audit_entries(),
            Self::DropSchemaDefaults(action) => action.audit_entries(),
            Self::DropUnsupportedRequestBodyOperations(action) => action.audit_entries(),
            Self::RewriteDeepObjectQueryParams(action) => action.audit_entries(),
            Self::DropOptionalObjectQueryParams(action) => action.audit_entries(),
            Self::DropSchemalessRequestBody(action) => action.audit_entries(),
            Self::DropEnumConstraint { target, .. } => vec![TransformAuditEntry::new(
                "typed_normalization",
                report.code,
                target.clone(),
                "drop enum constraint with colliding generated identifiers",
            )
            .with_action_kind(TransformActionKind::Drop)
            .with_backend_requirement_id("progenitor.schema.unique_sanitized_enum_variants")
            .with_backend_requirement("backend requires unique sanitized Rust enum variants")
            .with_before_after("constrained enum", "free-form string/schema")
            .with_before_after_json(
                json!({ "enum": "constrained" }),
                json!({ "enum": "removed" }),
            )],
            Self::ReplaceUnsupportedSchemaType { target, .. } => vec![TransformAuditEntry::new(
                "typed_normalization",
                report.code,
                target.clone(),
                "replace unsupported schema type with fallback",
            )
            .with_action_kind(TransformActionKind::Replace)
            .with_backend_requirement_id("progenitor.schema.supported_types")
            .with_backend_requirement("backend supports a limited set of OpenAPI schema types")
            .with_before_after("unsupported schema type", "fallback schema")
            .with_before_after_json(
                json!({ "type": "unsupported" }),
                json!({ "schema": "fallback" }),
            )],
            Self::DropCollidingProperties {
                target, dropped, ..
            } => vec![TransformAuditEntry::new(
                "typed_normalization",
                report.code,
                target.clone(),
                format!("drop colliding properties: {}", dropped.join(", ")),
            )
            .with_action_kind(TransformActionKind::Drop)
            .with_backend_requirement_id("progenitor.schema.unique_sanitized_object_properties")
            .with_backend_requirement("backend requires unique sanitized Rust field names")
            .with_before_after(
                "colliding object properties",
                "kept first property; removed collisions",
            )
            .with_before_after_json(
                json!({ "properties": "colliding" }),
                json!({ "dropped": dropped }),
            )],
        }
    }
}

pub(in crate::spec::normalize::progenitor_compatibility) fn content_target_label(
    target: &ContentTarget,
) -> String {
    match target {
        ContentTarget::ComponentRequestBody(name) => format!("component requestBody {name}"),
        ContentTarget::ComponentResponse(name) => format!("component response {name}"),
        ContentTarget::OperationRequestBody(operation) => {
            format!("operation {} requestBody", operation.label())
        }
        ContentTarget::OperationResponse { operation, status } => {
            format!("operation {} response {status}", operation.label())
        }
        ContentTarget::OperationDefaultResponse(operation) => {
            format!("operation {} default response", operation.label())
        }
    }
}

pub(in crate::spec::normalize::progenitor_compatibility) fn operation_target_pointer(
    target: &OperationTarget,
) -> String {
    format!(
        "/paths/{}/{}",
        json_pointer_escape(&target.path),
        target.method.to_ascii_lowercase()
    )
}

pub(in crate::spec::normalize::progenitor_compatibility) fn content_target_pointer(
    target: &ContentTarget,
) -> String {
    match target {
        ContentTarget::ComponentRequestBody(name) => {
            format!("/components/requestBodies/{}", json_pointer_escape(name))
        }
        ContentTarget::ComponentResponse(name) => {
            format!("/components/responses/{}", json_pointer_escape(name))
        }
        ContentTarget::OperationRequestBody(operation) => {
            format!("{}/requestBody", operation_target_pointer(operation))
        }
        ContentTarget::OperationResponse { operation, status } => format!(
            "{}/responses/{}",
            operation_target_pointer(operation),
            json_pointer_escape(status)
        ),
        ContentTarget::OperationDefaultResponse(operation) => {
            format!("{}/responses/default", operation_target_pointer(operation))
        }
    }
}

pub(in crate::spec::normalize::progenitor_compatibility) fn summarize_targets(
    targets: &[String],
) -> String {
    const MAX_INLINE_TARGETS: usize = 4;
    if targets.len() <= MAX_INLINE_TARGETS {
        targets.join(", ")
    } else {
        format!(
            "{} and {} more",
            targets[..MAX_INLINE_TARGETS].join(", "),
            targets.len() - MAX_INLINE_TARGETS
        )
    }
}
