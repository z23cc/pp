use std::{error::Error, fmt};

pub(super) const DIRECT_UNSUPPORTED_PREFIX: &str = "MCP direct HTTP invocation does not support";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DirectInvocationDiagnostic {
    pub code: &'static str,
    pub detail: String,
    pub source_code: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DirectInvocationUnsupported {
    diagnostic: DirectInvocationDiagnostic,
}

impl DirectInvocationUnsupported {
    pub(super) fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            diagnostic: DirectInvocationDiagnostic {
                code,
                detail: detail.into(),
                source_code: None,
            },
        }
    }

    pub(super) fn with_source_code(
        code: &'static str,
        detail: impl Into<String>,
        source_code: &'static str,
    ) -> Self {
        Self {
            diagnostic: DirectInvocationDiagnostic {
                code,
                detail: detail.into(),
                source_code: Some(source_code),
            },
        }
    }

    #[allow(dead_code)]
    pub(super) fn diagnostic(&self) -> &DirectInvocationDiagnostic {
        &self.diagnostic
    }

    pub(super) fn code(&self) -> &'static str {
        self.diagnostic.code
    }
}

impl fmt::Display for DirectInvocationUnsupported {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{DIRECT_UNSUPPORTED_PREFIX} {}",
            self.diagnostic.detail
        )
    }
}

impl Error for DirectInvocationUnsupported {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_invocation_unsupported_preserves_display_and_exposes_codes() {
        let unsupported = DirectInvocationUnsupported::with_source_code(
            "direct_http.parameter_schema_unsupported",
            "parameter 'q' is unsupported",
            "schema.keyword_unsupported",
        );

        assert_eq!(
            unsupported.to_string(),
            "MCP direct HTTP invocation does not support parameter 'q' is unsupported"
        );
        assert_eq!(
            unsupported.code(),
            "direct_http.parameter_schema_unsupported"
        );
        assert_eq!(
            unsupported.diagnostic().source_code,
            Some("schema.keyword_unsupported")
        );
    }
}
