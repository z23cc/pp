use openapiv3::MediaType;
use serde_json::json;

use super::{
    actions::{content_target_label, content_target_pointer, ContentTarget},
    content_report_subject, JSON_MIME,
};
use crate::backend::BackendCapabilities;
use crate::spec::normalization_rules::{self as rules, typed};
use crate::spec::report::ReportEntry;
use crate::spec::transform::{TransformActionKind, TransformAuditEntry};

#[derive(Debug, Clone)]
pub(super) struct Action {
    target: ContentTarget,
    kept: String,
    report: ReportEntry,
}

impl Action {
    pub(super) fn new(
        target_label: &str,
        target: ContentTarget,
        kept: String,
        dropped: &[String],
    ) -> Self {
        Self {
            report: report(target_label, &target, &kept, dropped),
            target,
            kept,
        }
    }

    pub(super) fn report_entry(&self) -> &ReportEntry {
        &self.report
    }

    pub(super) fn target(&self) -> &ContentTarget {
        &self.target
    }

    pub(super) fn kept(&self) -> &str {
        &self.kept
    }

    pub(super) fn audit_entries(&self) -> Vec<TransformAuditEntry> {
        vec![TransformAuditEntry::new(
            "typed_normalization",
            self.report.code,
            content_target_label(&self.target),
            format!("prune content types to {}", self.kept),
        )
        .with_target_pointer(content_target_pointer(&self.target))
        .with_action_kind(TransformActionKind::Prune)
        .with_backend_requirement_id("progenitor.content_type.single_supported")
        .with_backend_requirement("backend requires one supported content type per message")
        .with_before_after("multiple content types", format!("kept {}", self.kept))
        .with_before_after_json(
            json!({ "content_types": "multiple" }),
            json!({ "kept": self.kept }),
        )]
    }

    pub(super) fn apply_approved(
        &self,
        content: &mut indexmap::IndexMap<String, MediaType>,
        warnings: &mut Vec<ReportEntry>,
    ) {
        apply_pruning(content, &self.kept);
        warnings.push(self.report.clone());
    }
}

pub(super) fn propose_response(
    content: &indexmap::IndexMap<String, MediaType>,
    target_label: &str,
    target: ContentTarget,
    backend_capabilities: &BackendCapabilities,
) -> Option<Action> {
    let (kept, dropped) = propose_response_pruning(content, backend_capabilities)?;
    Some(Action::new(target_label, target, kept, &dropped))
}

pub(super) fn propose_request(
    content: &indexmap::IndexMap<String, MediaType>,
    target_label: &str,
    target: ContentTarget,
    backend_capabilities: &BackendCapabilities,
) -> Option<Action> {
    let (kept, dropped) = propose_request_pruning(content, backend_capabilities)?;
    Some(Action::new(target_label, target, kept, &dropped))
}

fn report(
    target_label: &str,
    target: &ContentTarget,
    kept: &str,
    dropped: &[String],
) -> ReportEntry {
    rules::typed_warning(
        typed::CONTENT_TYPES_PRUNED,
        format!(
            "normalized {target_label} — kept {kept}, dropped {}",
            dropped.join(", ")
        ),
        Some(content_report_subject(target, target_label)),
    )
}

fn propose_response_pruning(
    content: &indexmap::IndexMap<String, MediaType>,
    backend_capabilities: &BackendCapabilities,
) -> Option<(String, Vec<String>)> {
    if !backend_capabilities
        .message_content
        .requires_single_content_type_per_message
        || content.len() <= 1
    {
        return None;
    }

    let kept = if content.contains_key(JSON_MIME) {
        JSON_MIME.to_string()
    } else {
        content.keys().min().expect("content has entries").clone()
    };
    let dropped: Vec<String> = content
        .keys()
        .filter(|mime| *mime != &kept)
        .cloned()
        .collect();

    Some((kept, dropped))
}

fn propose_request_pruning(
    content: &indexmap::IndexMap<String, MediaType>,
    backend_capabilities: &BackendCapabilities,
) -> Option<(String, Vec<String>)> {
    let supported: Vec<String> = content
        .keys()
        .filter(|mime| is_supported_request_mime(mime, backend_capabilities))
        .cloned()
        .collect();
    if supported.is_empty() {
        return None;
    }
    if supported.len() == content.len()
        && (!backend_capabilities
            .message_content
            .requires_single_content_type_per_message
            || content.len() <= 1)
    {
        return None;
    }

    let kept = if content.contains_key(JSON_MIME) && supported.iter().any(|mime| mime == JSON_MIME)
    {
        JSON_MIME.to_string()
    } else {
        supported
            .into_iter()
            .min()
            .expect("supported content exists")
    };
    let dropped: Vec<String> = content
        .keys()
        .filter(|mime| *mime != &kept)
        .cloned()
        .collect();

    Some((kept, dropped))
}

fn apply_pruning(content: &mut indexmap::IndexMap<String, MediaType>, kept: &str) {
    let media_type = content
        .get(kept)
        .unwrap_or_else(|| panic!("approved content pruning target {kept} must exist"))
        .clone();
    content.clear();
    content.insert(kept.to_string(), media_type);
}

fn is_supported_request_mime(mime: &str, backend_capabilities: &BackendCapabilities) -> bool {
    backend_capabilities
        .request_bodies
        .supported_content_types
        .contains(&mime)
}
