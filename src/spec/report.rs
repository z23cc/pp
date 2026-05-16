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

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ReportEffect {
    /// Deterministic repair that does not intentionally remove API surface.
    LosslessRepair,
    /// User-requested selection such as operation slicing.
    ExplicitSelection,
    /// Compatibility rewrite that changes how a spec shape is represented.
    LossyRewrite,
    /// Compatibility change that removes source spec information or API surface.
    SemanticDrop,
    /// Workaround for a known codegen/backend limitation.
    BackendWorkaround,
    /// Last-resort replacement where pp cannot preserve source semantics.
    UnsafeFallback,
}

impl ReportEffect {
    pub fn allowed_without_compat_flag(self) -> bool {
        matches!(self, Self::LosslessRepair | Self::ExplicitSelection)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LosslessRepair => "lossless_repair",
            Self::ExplicitSelection => "explicit_selection",
            Self::LossyRewrite => "lossy_rewrite",
            Self::SemanticDrop => "semantic_drop",
            Self::BackendWorkaround => "backend_workaround",
            Self::UnsafeFallback => "unsafe_fallback",
        }
    }
}

impl std::fmt::Display for ReportEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ReportEffect {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "lossless_repair" => Ok(Self::LosslessRepair),
            "explicit_selection" => Ok(Self::ExplicitSelection),
            "lossy_rewrite" => Ok(Self::LossyRewrite),
            "semantic_drop" => Ok(Self::SemanticDrop),
            "backend_workaround" => Ok(Self::BackendWorkaround),
            "unsafe_fallback" => Ok(Self::UnsafeFallback),
            _ => Err(format!(
                "unknown report effect '{value}' (expected one of: lossless_repair, explicit_selection, lossy_rewrite, semantic_drop, backend_workaround, unsafe_fallback)"
            )),
        }
    }
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
    pub effect: ReportEffect,
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
        effect: ReportEffect,
        code: &'static str,
        message: impl Into<String>,
        subject: Option<ReportSubject>,
    ) -> Self {
        Self {
            stage,
            severity: ReportSeverity::Warning,
            effect,
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
