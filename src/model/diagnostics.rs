use std::{error::Error, fmt};

pub(super) const DIRECT_UNSUPPORTED_PREFIX: &str = "MCP direct HTTP invocation does not support";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DirectInvocationUnsupported {
    detail: String,
}

impl DirectInvocationUnsupported {
    pub(super) fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for DirectInvocationUnsupported {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{DIRECT_UNSUPPORTED_PREFIX} {}", self.detail)
    }
}

impl Error for DirectInvocationUnsupported {}
