//! Transform planning and approval for spec preparation.
//!
//! The current implementation builds the plan from structured reports emitted by
//! preparation rules. The important interface is policy approval: callers decide
//! which effects or rule codes may pass before generated artifacts are produced.

use super::report::{ReportEffect, ReportEntry};
use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TransformPlan {
    pub entries: Vec<ReportEntry>,
    pub approval: Option<TransformApproval>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audits: Vec<TransformAuditEntry>,
}

impl TransformPlan {
    pub fn from_reports(reports: &[ReportEntry]) -> Self {
        Self::from_reports_with_audits(reports, Vec::new())
    }

    pub fn from_reports_with_audits(
        reports: &[ReportEntry],
        audits: Vec<TransformAuditEntry>,
    ) -> Self {
        Self {
            entries: reports.to_vec(),
            approval: None,
            audits,
        }
    }

    pub fn add_audits(&mut self, audits: impl IntoIterator<Item = TransformAuditEntry>) {
        self.audits.extend(audits);
    }

    pub fn approve(&mut self, policy: &TransformPolicy) -> Result<()> {
        let rejected = self.rejected(policy);
        if !rejected.is_empty() {
            let mut message = format!(
                "{} transform policy rejected {} compatibility transform(s)",
                policy.profile.as_str(),
                rejected.len()
            );
            for report in rejected.iter().take(8) {
                message.push_str(&format!(
                    "\n- {} [{}]: {}",
                    report.code,
                    report.effect,
                    report.formatted_warning()
                ));
            }
            if rejected.len() > 8 {
                message.push_str(&format!("\n- ... {} more", rejected.len() - 8));
            }
            message.push_str(
                "\nPass --allow-effect or --allow-report-code to approve explicit compatibility transforms.",
            );
            return Err(anyhow!(message));
        }

        self.approval = Some(TransformApproval::from_policy(policy, &self.entries));
        Ok(())
    }

    pub fn rejected<'a>(&'a self, policy: &TransformPolicy) -> Vec<&'a ReportEntry> {
        self.entries
            .iter()
            .filter(|entry| !policy.allows(entry))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TransformPolicy {
    pub profile: PolicyProfile,
    pub allowed_effects: BTreeSet<ReportEffect>,
    pub allowed_codes: BTreeSet<String>,
}

impl Default for TransformPolicy {
    fn default() -> Self {
        Self::strict()
    }
}

impl TransformPolicy {
    pub fn strict() -> Self {
        Self {
            profile: PolicyProfile::Strict,
            allowed_effects: BTreeSet::new(),
            allowed_codes: BTreeSet::new(),
        }
    }

    pub fn compatibility() -> Self {
        Self {
            profile: PolicyProfile::Compatibility,
            allowed_effects: BTreeSet::new(),
            allowed_codes: BTreeSet::new(),
        }
    }

    pub fn allow_effect(mut self, effect: ReportEffect) -> Self {
        self.allowed_effects.insert(effect);
        self
    }

    pub fn allow_code(mut self, code: impl Into<String>) -> Self {
        self.allowed_codes.insert(code.into());
        self
    }

    pub fn allows(&self, report: &ReportEntry) -> bool {
        self.allowed_by(report).is_some()
    }

    fn allowed_by(&self, report: &ReportEntry) -> Option<&'static str> {
        match self.profile {
            PolicyProfile::Compatibility => Some("compatibility_profile"),
            PolicyProfile::Strict => {
                if report.effect.allowed_without_compat_flag() {
                    Some("strict_default")
                } else if self.allowed_codes.contains(report.code) {
                    Some("report_code_allowlist")
                } else if self.allowed_effects.contains(&report.effect) {
                    Some("effect_allowlist")
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PolicyProfile {
    Strict,
    Compatibility,
}

impl PolicyProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Compatibility => "compatibility",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TransformActionKind {
    RawRepair,
    Rename,
    Prune,
    Drop,
    Rewrite,
    Replace,
    Relax,
    BackendSourceTransform,
    RuntimeBridge,
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
    pub target_pointer: Option<String>,
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
            target_pointer: None,
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

    pub fn with_target_pointer(mut self, value: impl Into<String>) -> Self {
        self.target_pointer = Some(value.into());
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

pub(crate) fn json_pointer_escape(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TransformApproval {
    pub profile: &'static str,
    pub allowed_effects: Vec<&'static str>,
    pub allowed_codes: Vec<String>,
    pub decisions: Vec<TransformDecision>,
}

impl TransformApproval {
    fn from_policy(policy: &TransformPolicy, reports: &[ReportEntry]) -> Self {
        Self {
            profile: policy.profile.as_str(),
            allowed_effects: policy
                .allowed_effects
                .iter()
                .map(|effect| effect.as_str())
                .collect(),
            allowed_codes: policy.allowed_codes.iter().cloned().collect(),
            decisions: reports
                .iter()
                .map(|report| TransformDecision {
                    code: report.code,
                    effect: report.effect.as_str(),
                    allowed_by: policy
                        .allowed_by(report)
                        .expect("approval is only recorded after policy accepts every report"),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TransformDecision {
    pub code: &'static str,
    pub effect: &'static str,
    pub allowed_by: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::report::{ReportEffect, ReportStage};

    #[test]
    fn strict_policy_rejects_semantic_drop_unless_effect_is_allowed() {
        let report = ReportEntry::warning(
            ReportStage::TypedNormalization,
            ReportEffect::SemanticDrop,
            "spec.test.drop",
            "dropped something",
            None,
        );
        let mut plan = TransformPlan::from_reports(&[report]);

        assert!(plan.approve(&TransformPolicy::strict()).is_err());
        assert!(plan
            .approve(&TransformPolicy::strict().allow_effect(ReportEffect::SemanticDrop))
            .is_ok());
    }

    #[test]
    fn strict_policy_can_allow_specific_report_code() {
        let report = ReportEntry::warning(
            ReportStage::TypedNormalization,
            ReportEffect::UnsafeFallback,
            "spec.test.unsafe",
            "unsafe replacement",
            None,
        );
        let mut plan = TransformPlan::from_reports(&[report]);

        assert!(plan
            .approve(&TransformPolicy::strict().allow_code("spec.test.unsafe"))
            .is_ok());
        let approval = plan.approval.expect("approval recorded");
        assert_eq!(approval.profile, "strict");
        assert_eq!(approval.allowed_codes, vec!["spec.test.unsafe".to_string()]);
        assert_eq!(approval.decisions[0].allowed_by, "report_code_allowlist");
    }
}
