//! Inventory of normalization report codes and their coarse rule groups.
//!
//! The inventory is intentionally small and static for now: it makes every
//! compatibility warning searchable and gives future rule modules a stable place
//! to declare which stage/group emitted a report.

use super::report::{ReportEffect, ReportEntry, ReportStage, ReportSubject};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleGroup {
    PreParseTolerance,
    OpenApiDowngrade,
    OperationNaming,
    ProgenitorCompatibility,
    ResponseRelaxation,
    Slicing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizationRule {
    pub code: &'static str,
    pub group: RuleGroup,
    pub effect: ReportEffect,
    pub summary: &'static str,
}

pub mod pre_parse {
    pub const OPENAPI_31_DOWNGRADED: &str = "spec.pre_parse.openapi_31_downgraded";
    pub const NUMERIC_BOUNDS_CLAMPED: &str = "spec.pre_parse.numeric_bounds_clamped";
    pub const TAG_DESCRIPTIONS_REPLACED: &str = "spec.pre_parse.tag_descriptions_replaced";
    pub const REF_ONLY_OPERATIONS_REPLACED: &str = "spec.pre_parse.ref_only_operations_replaced";
}

pub mod typed {
    pub const OPERATION_IDS_SHORTENED: &str = "spec.normalize.operation_ids_shortened";
    pub const SCHEMA_DEFAULTS_DROPPED: &str = "spec.normalize.schema_defaults_dropped";
    pub const UNSUPPORTED_REQUEST_BODIES_DROPPED: &str =
        "spec.normalize.unsupported_request_bodies_dropped";
    pub const DEEP_OBJECT_QUERY_PARAMS_REWRITTEN: &str =
        "spec.normalize.deep_object_query_params_rewritten";
    pub const RESPONSE_SCHEMAS_RELAXED: &str = "spec.normalize.response_schemas_relaxed";
    pub const OPTIONAL_OBJECT_QUERY_PARAMS_DROPPED: &str =
        "spec.normalize.optional_object_query_params_dropped";
    pub const SCHEMALESS_REQUEST_BODY_DROPPED: &str =
        "spec.normalize.schemaless_request_body_dropped";
    pub const RESPONSE_VARIANTS_PRUNED: &str = "spec.normalize.response_variants_pruned";
    pub const CONTENT_TYPES_PRUNED: &str = "spec.normalize.content_types_pruned";
    pub const ENUM_CONSTRAINT_DROPPED: &str = "spec.normalize.enum_constraint_dropped";
    pub const UNSUPPORTED_SCHEMA_TYPE_REPLACED: &str =
        "spec.normalize.unsupported_schema_type_replaced";
    pub const PROPERTIES_COLLIDING_DROPPED: &str = "spec.normalize.properties_colliding_dropped";
}

pub mod slicing {
    pub const OPERATIONS_FILTERED: &str = "spec.slice.operations_filtered";
    pub const COMPONENTS_PRUNED: &str = "spec.slice.components_pruned";
}

pub const RULES: &[NormalizationRule] = &[
    NormalizationRule {
        code: pre_parse::OPENAPI_31_DOWNGRADED,
        group: RuleGroup::OpenApiDowngrade,
        effect: ReportEffect::LossyRewrite,
        summary: "downgrade supported OpenAPI 3.1 shapes into the 3.0 parser path",
    },
    NormalizationRule {
        code: pre_parse::NUMERIC_BOUNDS_CLAMPED,
        group: RuleGroup::PreParseTolerance,
        effect: ReportEffect::LossyRewrite,
        summary: "clamp out-of-range numeric bounds before typed deserialization",
    },
    NormalizationRule {
        code: pre_parse::TAG_DESCRIPTIONS_REPLACED,
        group: RuleGroup::PreParseTolerance,
        effect: ReportEffect::LossyRewrite,
        summary: "replace non-string top-level tag descriptions with empty strings",
    },
    NormalizationRule {
        code: pre_parse::REF_ONLY_OPERATIONS_REPLACED,
        group: RuleGroup::PreParseTolerance,
        effect: ReportEffect::UnsafeFallback,
        summary: "replace ref-only operations with parseable placeholder operations",
    },
    NormalizationRule {
        code: typed::OPERATION_IDS_SHORTENED,
        group: RuleGroup::OperationNaming,
        effect: ReportEffect::LosslessRepair,
        summary: "shorten verbose operation IDs while preserving uniqueness",
    },
    NormalizationRule {
        code: typed::SCHEMA_DEFAULTS_DROPPED,
        group: RuleGroup::ProgenitorCompatibility,
        effect: ReportEffect::BackendWorkaround,
        summary: "drop schema defaults that typify/progenitor may reject",
    },
    NormalizationRule {
        code: typed::UNSUPPORTED_REQUEST_BODIES_DROPPED,
        group: RuleGroup::ProgenitorCompatibility,
        effect: ReportEffect::SemanticDrop,
        summary: "drop operations whose request body has no supported media type",
    },
    NormalizationRule {
        code: typed::DEEP_OBJECT_QUERY_PARAMS_REWRITTEN,
        group: RuleGroup::ProgenitorCompatibility,
        effect: ReportEffect::BackendWorkaround,
        summary: "rewrite unsupported deepObject query parameters to form style",
    },
    NormalizationRule {
        code: typed::RESPONSE_SCHEMAS_RELAXED,
        group: RuleGroup::ResponseRelaxation,
        effect: ReportEffect::BackendWorkaround,
        summary: "relax output-only response schemas for tolerant deserialization",
    },
    NormalizationRule {
        code: typed::OPTIONAL_OBJECT_QUERY_PARAMS_DROPPED,
        group: RuleGroup::ProgenitorCompatibility,
        effect: ReportEffect::SemanticDrop,
        summary: "drop optional object-shaped query params that panic builder generation",
    },
    NormalizationRule {
        code: typed::SCHEMALESS_REQUEST_BODY_DROPPED,
        group: RuleGroup::ProgenitorCompatibility,
        effect: ReportEffect::SemanticDrop,
        summary: "drop schemaless request bodies from generated CLI input",
    },
    NormalizationRule {
        code: typed::RESPONSE_VARIANTS_PRUNED,
        group: RuleGroup::ProgenitorCompatibility,
        effect: ReportEffect::SemanticDrop,
        summary: "keep one response variant per operation before codegen",
    },
    NormalizationRule {
        code: typed::CONTENT_TYPES_PRUNED,
        group: RuleGroup::ProgenitorCompatibility,
        effect: ReportEffect::SemanticDrop,
        summary: "keep one supported request/response content type before codegen",
    },
    NormalizationRule {
        code: typed::ENUM_CONSTRAINT_DROPPED,
        group: RuleGroup::ProgenitorCompatibility,
        effect: ReportEffect::SemanticDrop,
        summary: "drop enum constraints whose values collide after Rust identifier sanitization",
    },
    NormalizationRule {
        code: typed::UNSUPPORTED_SCHEMA_TYPE_REPLACED,
        group: RuleGroup::ProgenitorCompatibility,
        effect: ReportEffect::UnsafeFallback,
        summary: "replace unsupported schema type names with a fallback schema",
    },
    NormalizationRule {
        code: typed::PROPERTIES_COLLIDING_DROPPED,
        group: RuleGroup::ProgenitorCompatibility,
        effect: ReportEffect::SemanticDrop,
        summary: "drop object properties that collide after Rust field-name sanitization",
    },
    NormalizationRule {
        code: slicing::OPERATIONS_FILTERED,
        group: RuleGroup::Slicing,
        effect: ReportEffect::ExplicitSelection,
        summary: "filter operations according to slice options",
    },
    NormalizationRule {
        code: slicing::COMPONENTS_PRUNED,
        group: RuleGroup::Slicing,
        effect: ReportEffect::ExplicitSelection,
        summary: "prune components unreachable from the selected operations",
    },
];

fn rule_for_code(code: &'static str) -> &'static NormalizationRule {
    RULES
        .iter()
        .find(|rule| rule.code == code)
        .unwrap_or_else(|| panic!("unregistered normalization report code: {code}"))
}

fn assert_rule_group(code: &'static str, allowed: &[RuleGroup]) -> ReportEffect {
    let rule = rule_for_code(code);
    assert!(
        allowed.contains(&rule.group),
        "normalization report code {code} belongs to {:?}, expected one of {allowed:?}",
        rule.group
    );
    rule.effect
}

pub fn pre_parse_warning(
    code: &'static str,
    message: impl Into<String>,
    subject: Option<ReportSubject>,
) -> ReportEntry {
    let effect = assert_rule_group(
        code,
        &[RuleGroup::PreParseTolerance, RuleGroup::OpenApiDowngrade],
    );
    ReportEntry::warning(
        ReportStage::PreParseTolerance,
        effect,
        code,
        message,
        subject,
    )
}

pub fn typed_warning(
    code: &'static str,
    message: impl Into<String>,
    subject: Option<ReportSubject>,
) -> ReportEntry {
    let effect = assert_rule_group(
        code,
        &[
            RuleGroup::OperationNaming,
            RuleGroup::ProgenitorCompatibility,
            RuleGroup::ResponseRelaxation,
        ],
    );
    ReportEntry::warning(
        ReportStage::TypedNormalization,
        effect,
        code,
        message,
        subject,
    )
}

pub fn slicing_warning(
    code: &'static str,
    message: impl Into<String>,
    subject: Option<ReportSubject>,
) -> ReportEntry {
    let effect = assert_rule_group(code, &[RuleGroup::Slicing]);
    ReportEntry::warning(ReportStage::Slicing, effect, code, message, subject)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn rule_codes_are_unique_and_grouped() {
        let mut codes = HashSet::new();
        for rule in RULES {
            assert!(codes.insert(rule.code), "duplicate rule code {}", rule.code);
            assert!(!rule.summary.is_empty());
        }
        assert!(RULES
            .iter()
            .any(|rule| rule.group == RuleGroup::PreParseTolerance));
        assert!(RULES
            .iter()
            .any(|rule| rule.group == RuleGroup::OpenApiDowngrade));
        assert!(RULES
            .iter()
            .any(|rule| rule.group == RuleGroup::OperationNaming));
        assert!(RULES
            .iter()
            .any(|rule| rule.group == RuleGroup::ProgenitorCompatibility));
        assert!(RULES
            .iter()
            .any(|rule| rule.group == RuleGroup::ResponseRelaxation));
        assert!(RULES.iter().any(|rule| rule.group == RuleGroup::Slicing));
    }
}
