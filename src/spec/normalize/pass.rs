use openapiv3::OpenAPI;

use crate::backend::BackendCapabilities;
use crate::spec::report::ReportEntry;
use crate::spec::transform::TransformAuditEntry;

pub(super) struct NormalizationPassContext<'a> {
    pub(super) backend_capabilities: &'a BackendCapabilities,
}

impl<'a> NormalizationPassContext<'a> {
    pub(super) fn new(backend_capabilities: &'a BackendCapabilities) -> Self {
        Self {
            backend_capabilities,
        }
    }
}

pub(super) trait NormalizationPassPlan {
    fn report_entries(&self) -> Vec<ReportEntry>;
    fn audit_entries(&self) -> Vec<TransformAuditEntry>;
}

pub(super) trait NormalizationPass {
    type Plan: NormalizationPassPlan;

    fn propose(&self, spec: &OpenAPI, context: &NormalizationPassContext<'_>) -> Self::Plan;

    fn apply_approved(
        &self,
        spec: &mut OpenAPI,
        reports: &mut Vec<ReportEntry>,
        context: &NormalizationPassContext<'_>,
        approved_plan: &Self::Plan,
    );
}
