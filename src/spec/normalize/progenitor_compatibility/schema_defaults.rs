use openapiv3::Schema;
use serde_json::json;

use crate::backend::BackendCapabilities;
use crate::spec::normalization_rules::{self as rules, typed};
use crate::spec::report::ReportEntry;
use crate::spec::transform::{TransformActionKind, TransformAuditEntry};

#[derive(Debug, Clone)]
pub(super) struct Action {
    targets: Vec<String>,
    report: ReportEntry,
}

impl Action {
    pub(super) fn new(targets: Vec<String>) -> Self {
        Self {
            report: summary_report(targets.len()),
            targets,
        }
    }

    pub(super) fn report_entry(&self) -> &ReportEntry {
        &self.report
    }

    pub(super) fn audit_entries(&self) -> Vec<TransformAuditEntry> {
        vec![TransformAuditEntry::new(
            "typed_normalization",
            self.report.code,
            summarize_targets(&self.targets),
            format!("drop default values from {} schemas", self.targets.len()),
        )
        .with_action_kind(TransformActionKind::Drop)
        .with_backend_requirement_id("progenitor.schema.defaults_unsupported")
        .with_backend_requirement(
            "backend/typify path does not accept schema default values reliably",
        )
        .with_before_after("schema.default present", "schema.default removed")
        .with_before_after_json(
            json!({ "default": "present" }),
            json!({ "default": "removed" }),
        )]
    }

    pub(super) fn contains(&self, path: &str) -> bool {
        self.targets.iter().any(|target| target == path)
    }
}

pub(super) fn should_propose(schema: &Schema, backend_capabilities: &BackendCapabilities) -> bool {
    !backend_capabilities.schemas.supports_defaults && schema.schema_data.default.is_some()
}

pub(super) fn apply(schema: &mut Schema, should_drop: bool) -> bool {
    should_drop && schema.schema_data.default.take().is_some()
}

pub(super) fn summary_report(count: usize) -> ReportEntry {
    rules::typed_warning(
        typed::SCHEMA_DEFAULTS_DROPPED,
        format!("normalized {count} schemas — dropped default values"),
        None,
    )
}

fn summarize_targets(targets: &[String]) -> String {
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
