use serde_json::json;

use crate::spec::normalization_rules::{self as rules, typed};
use crate::spec::report::ReportEntry;
use crate::spec::transform::{TransformActionKind, TransformAuditEntry};

use super::OperationTarget;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Target {
    pub(super) operation: OperationTarget,
    pub(super) param_name: String,
    pub(super) label: String,
}

#[derive(Debug, Clone)]
pub(super) struct DeepObjectAction {
    targets: Vec<Target>,
    report: ReportEntry,
}

impl DeepObjectAction {
    pub(super) fn new(targets: Vec<Target>) -> Self {
        let labels = targets
            .iter()
            .map(|target| target.label.clone())
            .collect::<Vec<_>>();
        Self {
            report: deep_object_query_params_report(&labels),
            targets,
        }
    }

    pub(super) fn report_entry(&self) -> &ReportEntry {
        &self.report
    }

    pub(super) fn targets(&self) -> &[Target] {
        &self.targets
    }

    pub(super) fn labels(&self) -> Vec<String> {
        self.targets
            .iter()
            .map(|target| target.label.clone())
            .collect()
    }

    pub(super) fn audit_entries(&self) -> Vec<TransformAuditEntry> {
        self.targets
            .iter()
            .map(|target| {
                TransformAuditEntry::new(
                    "typed_normalization",
                    self.report.code,
                    format!("{}.query.{}", target.operation.label(), target.param_name),
                    "rewrite query parameter style",
                )
                .with_action_kind(TransformActionKind::Rewrite)
                .with_backend_requirement_id("progenitor.query.deep_object_unsupported")
                .with_backend_requirement("backend does not support deepObject query parameters")
                .with_before_after("style: deepObject", "style: form")
                .with_before_after_json(
                    json!({ "style": "deepObject" }),
                    json!({ "style": "form" }),
                )
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub(super) struct OptionalObjectAction {
    targets: Vec<Target>,
    report: ReportEntry,
}

impl OptionalObjectAction {
    pub(super) fn new(targets: Vec<Target>) -> Self {
        let labels = targets
            .iter()
            .map(|target| target.label.clone())
            .collect::<Vec<_>>();
        Self {
            report: optional_object_query_params_report(&labels),
            targets,
        }
    }

    pub(super) fn report_entry(&self) -> &ReportEntry {
        &self.report
    }

    pub(super) fn targets(&self) -> &[Target] {
        &self.targets
    }

    pub(super) fn labels(&self) -> Vec<String> {
        self.targets
            .iter()
            .map(|target| target.label.clone())
            .collect()
    }

    pub(super) fn audit_entries(&self) -> Vec<TransformAuditEntry> {
        self.targets
            .iter()
            .map(|target| {
                TransformAuditEntry::new(
                    "typed_normalization",
                    self.report.code,
                    format!("{}.query.{}", target.operation.label(), target.param_name),
                    "drop optional object-shaped query parameter",
                )
                .with_action_kind(TransformActionKind::Drop)
                .with_backend_requirement_id("progenitor.query.optional_object_unsupported")
                .with_backend_requirement(
                    "backend builder shape does not support optional object query parameters",
                )
                .with_before_after("optional object query parameter", "parameter removed")
                .with_before_after_json(json!({ "parameter": &target.param_name }), json!(null))
            })
            .collect()
    }
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
