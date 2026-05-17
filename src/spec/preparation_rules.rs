//! Inventory of preparation report codes.
//!
//! This inventory covers explicit slicing reports only.

use super::report::{ReportEffect, ReportEntry, ReportStage, ReportSubject};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleGroup {
    Slicing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparationRule {
    pub code: &'static str,
    pub group: RuleGroup,
    pub effect: ReportEffect,
    pub summary: &'static str,
}

pub mod slicing {
    pub const OPERATIONS_FILTERED: &str = "spec.slice.operations_filtered";
    pub const COMPONENTS_PRUNED: &str = "spec.slice.components_pruned";
}

pub const RULES: &[PreparationRule] = &[
    PreparationRule {
        code: slicing::OPERATIONS_FILTERED,
        group: RuleGroup::Slicing,
        effect: ReportEffect::ExplicitSelection,
        summary: "filter operations according to slice options",
    },
    PreparationRule {
        code: slicing::COMPONENTS_PRUNED,
        group: RuleGroup::Slicing,
        effect: ReportEffect::ExplicitSelection,
        summary: "prune components unreachable from the selected operations",
    },
];

fn rule_for_code(code: &'static str) -> &'static PreparationRule {
    RULES
        .iter()
        .find(|rule| rule.code == code)
        .unwrap_or_else(|| panic!("unregistered preparation report code: {code}"))
}

fn assert_rule_group(code: &'static str, allowed: &[RuleGroup]) -> ReportEffect {
    let rule = rule_for_code(code);
    assert!(
        allowed.contains(&rule.group),
        "preparation report code {code} belongs to {:?}, expected one of {allowed:?}",
        rule.group
    );
    rule.effect
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
        assert!(RULES.iter().all(|rule| rule.group == RuleGroup::Slicing));
    }
}
