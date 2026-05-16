//! Structured reports emitted while preparing an OpenAPI spec.
//!
//! These reports are internal for now. User-facing compatibility is preserved
//! by formatting each warning report back to its original message at CLI and
//! pipeline boundaries.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportStage {
    PreParseTolerance,
    TypedNormalization,
    Slicing,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportSeverity {
    Warning,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ReportSubject {
    Operation(String),
    Schema(String),
    Component(String),
}

impl ReportSubject {
    pub fn operation(value: impl Into<String>) -> Self {
        Self::Operation(value.into())
    }

    pub fn schema(value: impl Into<String>) -> Self {
        Self::Schema(value.into())
    }

    pub fn component(value: impl Into<String>) -> Self {
        Self::Component(value.into())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReportEntry {
    pub stage: ReportStage,
    pub severity: ReportSeverity,
    pub code: &'static str,
    pub message: String,
    pub subject: Option<ReportSubject>,
}

impl std::ops::Deref for ReportEntry {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

impl PartialEq<&str> for ReportEntry {
    fn eq(&self, other: &&str) -> bool {
        self.message == *other
    }
}

impl PartialEq<ReportEntry> for &str {
    fn eq(&self, other: &ReportEntry) -> bool {
        *self == other.message
    }
}

impl ReportEntry {
    pub fn warning(
        stage: ReportStage,
        code: &'static str,
        message: impl Into<String>,
        subject: Option<ReportSubject>,
    ) -> Self {
        Self {
            stage,
            severity: ReportSeverity::Warning,
            code,
            message: message.into(),
            subject,
        }
    }

    pub fn formatted_warning(&self) -> &str {
        &self.message
    }
}

pub fn formatted_warnings(reports: &[ReportEntry]) -> Vec<String> {
    reports
        .iter()
        .filter(|report| report.severity == ReportSeverity::Warning)
        .map(|report| report.formatted_warning().to_string())
        .collect()
}
