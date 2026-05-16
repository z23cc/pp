use heck::ToSnakeCase;
use openapiv3::OpenAPI;
use std::collections::HashMap;

use crate::spec::normalization_rules::{self as rules, typed};
use crate::spec::report::{ReportEntry, ReportSubject};
use crate::spec::transform::TransformAuditEntry;
use crate::spec::traversal;

const VERBOSE_OPERATION_PREFIXES: &[&str] = &[
    "plausible_web_plugins_api_controllers_",
    "PlausibleWeb.Plugins.API.Controllers.",
    "application_controllers_",
];

#[derive(Debug, Clone, Default)]
pub(super) struct OperationNamingPlan {
    actions: Vec<OperationNamingAction>,
}

impl OperationNamingPlan {
    pub(super) fn report_entries(&self) -> Vec<ReportEntry> {
        self.actions
            .iter()
            .map(|action| action.report.clone())
            .collect()
    }

    pub(super) fn audit_entries(&self) -> Vec<TransformAuditEntry> {
        self.actions
            .iter()
            .map(OperationNamingAction::audit_entry)
            .collect()
    }
}

#[derive(Debug, Clone)]
struct OperationNamingAction {
    method: String,
    path: String,
    old: String,
    new: String,
    report: ReportEntry,
}

impl OperationNamingAction {
    fn audit_entry(&self) -> TransformAuditEntry {
        TransformAuditEntry::new(
            "typed_normalization",
            self.report.code,
            format!("operation {} {} operationId", self.method, self.path),
            "shorten operationId",
        )
        .with_before_after(&self.old, &self.new)
    }
}

pub(super) fn propose(spec: &OpenAPI) -> OperationNamingPlan {
    let ids = operation_ids(spec);
    let candidates: Vec<_> = ids
        .iter()
        .filter_map(|old| {
            shorten_candidate(old).map(|new| (old.clone(), new, last_segments(old, 2)))
        })
        .collect();
    let last_three_counts = count_by(candidates.iter().map(|(_, new, _)| new.clone()));
    let chosen: Vec<_> = candidates
        .into_iter()
        .map(|(old, last_three, last_two)| {
            let new = match last_three_counts.get(&last_three) {
                Some(1) => last_three,
                _ => last_two,
            };
            (old, new)
        })
        .collect();
    let chosen_counts = count_by(chosen.iter().map(|(_, new)| new.clone()));
    let replacements: HashMap<_, _> = chosen
        .into_iter()
        .filter(|(old, new)| old != new && chosen_counts.get(new) == Some(&1))
        .collect();

    let actions = traversal::operations(spec)
        .into_iter()
        .filter_map(|operation_ref| {
            let old = operation_ref.operation.operation_id.clone()?;
            let new = replacements.get(&old)?.clone();
            Some(OperationNamingAction {
                method: operation_ref.method.to_string(),
                path: operation_ref.path.to_string(),
                report: shortened_report(&old, &new),
                old,
                new,
            })
        })
        .collect();

    OperationNamingPlan { actions }
}

pub(super) fn apply_approved(
    spec: &mut OpenAPI,
    reports: &mut Vec<ReportEntry>,
    approved_plan: &OperationNamingPlan,
) {
    traversal::visit_operations_mut(spec, |operation_ref| {
        let Some(old) = operation_ref.operation.operation_id.clone() else {
            return;
        };
        let Some(action) = approved_plan.actions.iter().find(|action| {
            action.method == operation_ref.method
                && action.path == operation_ref.path
                && action.old == old
        }) else {
            return;
        };
        operation_ref.operation.operation_id = Some(action.new.clone());
        reports.push(action.report.clone());
    });
}

fn shortened_report(old: &str, new: &str) -> ReportEntry {
    rules::typed_warning(
        typed::OPERATION_IDS_SHORTENED,
        format!("shortened operation '{old}' → '{new}'"),
        Some(ReportSubject::operation(old)),
    )
}

fn operation_ids(spec: &OpenAPI) -> Vec<String> {
    traversal::operations(spec)
        .into_iter()
        .filter_map(|op| op.operation.operation_id.clone())
        .collect()
}

fn shorten_candidate(operation_id: &str) -> Option<String> {
    VERBOSE_OPERATION_PREFIXES
        .iter()
        .find_map(|prefix| {
            operation_id
                .strip_prefix(prefix)
                .map(|stripped| stripped.to_snake_case())
        })
        .or_else(|| {
            (operation_segments(operation_id).len() > 4).then(|| last_segments(operation_id, 3))
        })
}
fn last_segments(operation_id: &str, count: usize) -> String {
    let segments = operation_segments(operation_id);
    segments[segments.len().saturating_sub(count)..]
        .join("_")
        .to_snake_case()
}

fn operation_segments(operation_id: &str) -> Vec<&str> {
    operation_id.split(['_', '.']).collect()
}

fn count_by(values: impl Iterator<Item = String>) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    values.for_each(|value| *counts.entry(value).or_insert(0) += 1);
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use openapiv3::{OpenAPI, ReferenceOr};

    #[test]
    fn verbose_operation_ids_are_shortened() {
        assert_eq!(
            shorten_candidate("foo_bar_baz_qux_quux_widget_get").as_deref(),
            Some("quux_widget_get")
        );
        assert_eq!(
            shorten_candidate("PlausibleWeb.Plugins.API.Controllers.Capabilities.index").as_deref(),
            Some("capabilities_index")
        );
    }

    #[test]
    fn verbose_operation_id_shortening_emits_report() {
        let mut spec: OpenAPI = serde_yaml::from_str(
            r#"
openapi: 3.0.0
info:
  title: Verbose Operation
  version: "1.0.0"
paths:
  /capabilities:
    get:
      operationId: PlausibleWeb.Plugins.API.Controllers.Capabilities.index
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();

        let plan = propose(&spec);
        let mut warnings = Vec::new();
        apply_approved(&mut spec, &mut warnings, &plan);
        let path = spec.paths.paths.get("/capabilities").unwrap();
        let ReferenceOr::Item(path) = path else {
            panic!("expected inline path item");
        };

        assert_eq!(
            path.get.as_ref().unwrap().operation_id.as_deref(),
            Some("capabilities_index")
        );
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, typed::OPERATION_IDS_SHORTENED);
        assert_eq!(
            warnings[0].subject,
            Some(ReportSubject::operation(
                "PlausibleWeb.Plugins.API.Controllers.Capabilities.index"
            ))
        );
    }
}
