use openapiv3::{MediaType, RequestBody};
use serde_json::json;

use super::{
    actions::{
        content_target_label, content_target_pointer, operation_target_pointer, summarize_targets,
        ContentTarget, OperationRequestBodyDropTarget, OperationTarget,
    },
    content_report_subject,
};
use crate::backend::BackendCapabilities;
use crate::spec::normalization_rules::{self as rules, typed};
use crate::spec::report::{ReportEntry, ReportSubject};
use crate::spec::transform::{TransformActionKind, TransformAuditEntry};

#[derive(Debug, Clone)]
pub(super) struct UnsupportedAction {
    target: ContentTarget,
    report: ReportEntry,
}

impl UnsupportedAction {
    pub(super) fn new(
        target_label: &str,
        target: ContentTarget,
        content: &indexmap::IndexMap<String, MediaType>,
    ) -> Self {
        Self {
            report: unsupported_report(target_label, &target, content),
            target,
        }
    }

    pub(super) fn report_entry(&self) -> &ReportEntry {
        &self.report
    }

    pub(super) fn target(&self) -> &ContentTarget {
        &self.target
    }

    pub(super) fn audit_entries(&self) -> Vec<TransformAuditEntry> {
        vec![TransformAuditEntry::new(
            "typed_normalization",
            self.report.code,
            content_target_label(&self.target),
            "drop requestBody with only unsupported content types",
        )
        .with_target_pointer(content_target_pointer(&self.target))
        .with_action_kind(TransformActionKind::Drop)
        .with_backend_requirement_id("progenitor.request_body.supported_content_type")
        .with_backend_requirement("backend request body content-type support is limited")
        .with_before_after(
            "requestBody with unsupported content",
            "requestBody removed",
        )
        .with_before_after_json(json!({ "requestBody": "unsupported_content" }), json!(null))]
    }

    pub(super) fn apply_approved(
        &self,
        request_body: &mut RequestBody,
        content_target: &ContentTarget,
        warnings: &mut Vec<ReportEntry>,
    ) -> bool {
        request_body.content.clear();
        if matches!(content_target, ContentTarget::ComponentRequestBody(_)) {
            warnings.push(self.report.clone());
        }
        true
    }
}

#[derive(Debug, Clone)]
pub(super) struct UnsupportedOperationsAction {
    targets: Vec<OperationRequestBodyDropTarget>,
    report: ReportEntry,
}

impl UnsupportedOperationsAction {
    pub(super) fn new(targets: Vec<OperationRequestBodyDropTarget>) -> Self {
        let op_names = targets
            .iter()
            .map(|target| target.op_name.clone())
            .collect::<Vec<_>>();
        Self {
            report: unsupported_operations_report(&op_names),
            targets,
        }
    }

    pub(super) fn report_entry(&self) -> &ReportEntry {
        &self.report
    }

    pub(super) fn targets(&self) -> &[OperationRequestBodyDropTarget] {
        &self.targets
    }

    pub(super) fn target_for(
        &self,
        method: &str,
        path: &str,
    ) -> Option<&OperationRequestBodyDropTarget> {
        self.targets
            .iter()
            .find(|target| target.operation.method == method && target.operation.path == path)
    }

    pub(super) fn audit_entries(&self) -> Vec<TransformAuditEntry> {
        vec![TransformAuditEntry::new(
            "typed_normalization",
            self.report.code,
            summarize_targets(
                &self
                    .targets
                    .iter()
                    .map(|target| target.operation.label())
                    .collect::<Vec<_>>(),
            ),
            format!(
                "drop {} operations with unsupported request bodies",
                self.targets.len()
            ),
        )
        .with_action_kind(TransformActionKind::Drop)
        .with_backend_requirement_id("progenitor.operation.request_body_supported_content_type")
        .with_backend_requirement(
            "backend cannot generate operations whose request body has no supported media type",
        )
        .with_before_after("operation requestBody unsupported", "operation removed")
        .with_before_after_json(
            json!({ "operation": "requestBody unsupported" }),
            json!(null),
        )]
    }
}

#[derive(Debug, Clone)]
pub(super) struct SchemalessAction {
    target: OperationTarget,
    report: ReportEntry,
}

impl SchemalessAction {
    pub(super) fn new(target: OperationTarget, op_name: &str) -> Self {
        Self {
            target,
            report: schemaless_report(op_name),
        }
    }

    pub(super) fn report_entry(&self) -> &ReportEntry {
        &self.report
    }

    pub(super) fn matches(&self, operation: &OperationTarget) -> bool {
        &self.target == operation
    }

    pub(super) fn audit_entries(&self) -> Vec<TransformAuditEntry> {
        vec![TransformAuditEntry::new(
            "typed_normalization",
            self.report.code,
            format!("operation {} requestBody", self.target.label()),
            "drop schemaless requestBody",
        )
        .with_target_pointer(format!(
            "{}/requestBody",
            operation_target_pointer(&self.target)
        ))
        .with_action_kind(TransformActionKind::Drop)
        .with_backend_requirement_id("progenitor.request_body.schema_required")
        .with_backend_requirement("backend requires request body content to declare schemas")
        .with_before_after("requestBody content without schema", "requestBody removed")
        .with_before_after_json(json!({ "requestBody": "schemaless" }), json!(null))]
    }
}

pub(super) fn has_schemaless_content(request_body: &RequestBody) -> bool {
    request_body
        .content
        .values()
        .any(|media_type| media_type.schema.is_none())
}

pub(super) fn has_only_unsupported_types(
    content: &indexmap::IndexMap<String, MediaType>,
    backend_capabilities: &BackendCapabilities,
) -> bool {
    !content.is_empty()
        && content
            .keys()
            .all(|mime| !is_supported_request_mime(mime, backend_capabilities))
}

fn unsupported_report(
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

fn unsupported_operations_report(op_names: &[String]) -> ReportEntry {
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

fn schemaless_report(op_name: &str) -> ReportEntry {
    rules::typed_warning(
        typed::SCHEMALESS_REQUEST_BODY_DROPPED,
        format!("normalized {op_name} — dropped requestBody (no schema specified)"),
        Some(ReportSubject::operation(op_name)),
    )
}

fn is_supported_request_mime(mime: &str, backend_capabilities: &BackendCapabilities) -> bool {
    backend_capabilities
        .request_bodies
        .supported_content_types
        .contains(&mime)
}
