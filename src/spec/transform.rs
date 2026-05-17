//! Transform planning and audit records for spec preparation.
//!
//! The strict preparation path records structured reports and audit entries for
//! transparency. It does not approve, relax, or mutate typed OpenAPI.

use super::report::ReportEntry;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TransformPlan {
    pub entries: Vec<ReportEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audits: Vec<TransformAuditEntry>,
}

impl TransformPlan {
    pub fn from_reports_with_audits(
        reports: &[ReportEntry],
        audits: Vec<TransformAuditEntry>,
    ) -> Self {
        Self {
            entries: reports.to_vec(),
            audits,
        }
    }

    pub fn add_audits(&mut self, audits: impl IntoIterator<Item = TransformAuditEntry>) {
        self.audits.extend(audits);
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TransformActionKind {
    RuntimeDirectInvocation,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct TransformAuditEntry {
    pub source_stage: &'static str,
    pub code: String,
    pub target: String,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_requirement: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_kind: Option<TransformActionKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_requirement_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_json: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_json: Option<Value>,
}

impl TransformAuditEntry {
    pub fn new(
        source_stage: &'static str,
        code: impl Into<String>,
        target: impl Into<String>,
        action: impl Into<String>,
    ) -> Self {
        Self {
            source_stage,
            code: code.into(),
            target: target.into(),
            action: action.into(),
            backend_requirement: None,
            before: None,
            after: None,
            action_kind: None,
            backend_requirement_id: None,
            before_json: None,
            after_json: None,
        }
    }

    pub fn with_backend_requirement(mut self, value: impl Into<String>) -> Self {
        self.backend_requirement = Some(value.into());
        self
    }

    pub fn with_action_kind(mut self, value: TransformActionKind) -> Self {
        self.action_kind = Some(value);
        self
    }

    pub fn with_backend_requirement_id(mut self, value: impl Into<String>) -> Self {
        self.backend_requirement_id = Some(value.into());
        self
    }

    pub fn with_before_after(
        mut self,
        before: impl Into<String>,
        after: impl Into<String>,
    ) -> Self {
        self.before = Some(before.into());
        self.after = Some(after.into());
        self
    }

    pub fn with_before_after_json(mut self, before: Value, after: Value) -> Self {
        self.before_json = Some(before);
        self.after_json = Some(after);
        self
    }
}
