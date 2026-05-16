use openapiv3::{Operation, StatusCode};

use super::{operation_target_pointer, OperationTarget};
use crate::backend::BackendCapabilities;
use crate::spec::normalization_rules::{self as rules, typed};
use crate::spec::report::{ReportEntry, ReportSubject};
use crate::spec::transform::{TransformActionKind, TransformAuditEntry};
use serde_json::json;

#[derive(Debug, Clone)]
pub(super) struct Action {
    target: OperationTarget,
    kept: String,
    report: ReportEntry,
}

impl Action {
    pub(super) fn report_entry(&self) -> &ReportEntry {
        &self.report
    }

    pub(super) fn audit_entries(&self) -> Vec<TransformAuditEntry> {
        vec![TransformAuditEntry::new(
            "typed_normalization",
            self.report.code,
            format!("operation {} responses", self.target.label()),
            format!("prune response variants to {}", self.kept),
        )
        .with_target_pointer(format!(
            "{}/responses",
            operation_target_pointer(&self.target)
        ))
        .with_action_kind(TransformActionKind::Prune)
        .with_backend_requirement_id("progenitor.response.single_variant")
        .with_backend_requirement("backend requires one response variant per operation")
        .with_before_after("multiple response variants", format!("kept {}", self.kept))
        .with_before_after_json(
            json!({ "variants": "multiple" }),
            json!({ "kept": self.kept }),
        )]
    }

    pub(super) fn kept(&self) -> &str {
        &self.kept
    }

    pub(super) fn matches(&self, method: &str, path: &str) -> bool {
        self.target.method == method && self.target.path == path
    }

    pub(super) fn apply_approved(
        &self,
        operation: &mut Operation,
        warnings: &mut Vec<ReportEntry>,
    ) {
        operation
            .responses
            .responses
            .retain(|code, _| code.to_string() == self.kept);
        if self.kept != "default" {
            operation.responses.default = None;
        }
        warnings.push(self.report.clone());
    }
}

pub(super) fn propose(
    operation: &Operation,
    op_name: &str,
    target: OperationTarget,
    backend_capabilities: &BackendCapabilities,
) -> Option<Action> {
    let (kept, dropped) = propose_pruning(operation, backend_capabilities)?;
    Some(Action {
        target,
        report: report(op_name, &kept, &dropped),
        kept,
    })
}

fn propose_pruning(
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
    if !backend_capabilities
        .responses
        .requires_single_variant_per_operation
        || codes.len() <= 1
    {
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

fn report(op_name: &str, kept: &str, dropped: &[String]) -> ReportEntry {
    rules::typed_warning(
        typed::RESPONSE_VARIANTS_PRUNED,
        format!(
            "normalized {op_name} responses — kept {kept}, dropped {}",
            dropped.join(", ")
        ),
        Some(ReportSubject::operation(op_name)),
    )
}
