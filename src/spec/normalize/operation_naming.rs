use heck::ToSnakeCase;
use openapiv3::{OpenAPI, Operation, ReferenceOr};
use std::collections::HashMap;

use crate::spec::normalization_rules::{self as rules, typed};
use crate::spec::report::{ReportEntry, ReportSubject};

const VERBOSE_OPERATION_PREFIXES: &[&str] = &[
    "plausible_web_plugins_api_controllers_",
    "PlausibleWeb.Plugins.API.Controllers.",
    "application_controllers_",
];

pub(super) fn apply(spec: &mut OpenAPI, reports: &mut Vec<ReportEntry>) {
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

    for operation in operations_mut(spec) {
        if let Some(old) = operation.operation_id.clone() {
            if let Some(new) = replacements.get(&old) {
                operation.operation_id = Some(new.clone());
                reports.push(rules::typed_warning(
                    typed::OPERATION_IDS_SHORTENED,
                    format!("shortened operation '{old}' → '{new}'"),
                    Some(ReportSubject::operation(old)),
                ));
            }
        }
    }
}

fn operation_ids(spec: &OpenAPI) -> Vec<String> {
    spec.paths
        .iter()
        .filter_map(|(_, path_item)| match path_item {
            ReferenceOr::Item(item) => Some(item),
            ReferenceOr::Reference { .. } => None,
        })
        .flat_map(|item| {
            [
                &item.get,
                &item.put,
                &item.post,
                &item.delete,
                &item.options,
                &item.head,
                &item.patch,
                &item.trace,
            ]
        })
        .flatten()
        .filter_map(|op| op.operation_id.clone())
        .collect()
}

fn operations_mut(spec: &mut OpenAPI) -> Vec<&mut Operation> {
    spec.paths
        .paths
        .iter_mut()
        .filter_map(|(_, path_item)| match path_item {
            ReferenceOr::Item(item) => Some(item),
            ReferenceOr::Reference { .. } => None,
        })
        .flat_map(|item| {
            [
                &mut item.get,
                &mut item.put,
                &mut item.post,
                &mut item.delete,
                &mut item.options,
                &mut item.head,
                &mut item.patch,
                &mut item.trace,
            ]
        })
        .flatten()
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
    use openapiv3::OpenAPI;

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

        let mut warnings = Vec::new();
        apply(&mut spec, &mut warnings);
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
