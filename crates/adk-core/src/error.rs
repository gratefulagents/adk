use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Machine-readable failure categories; retryability is adapter advice, not
/// implied by a category (a provider failure may be permanent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Cancelled,
    DeadlineExceeded,
    InvalidInput,
    PermissionDenied,
    ApprovalDenied,
    Unsupported,
    Provider,
    ModelBehavior,
    Tool,
    Host,
    Guardrail,
    MaxTurns,
    Internal,
}

/// Serializable, host-safe error detail. Never include credentials or raw secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorInfo {
    pub category: ErrorCategory,
    pub message: String,
}

/// An operation failure with an optional non-serialized underlying cause.
#[derive(Debug, thiserror::Error)]
#[error("{message}", message = .info.message)]
pub struct Error {
    pub info: ErrorInfo,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl Error {
    /// Construct a categorized error without a cause.
    pub fn new(category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            info: ErrorInfo {
                category,
                message: message.into(),
            },
            source: None,
        }
    }

    /// Preserve the original error for diagnostics, outside serialized events.
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }
}

/// A failed run retains every available item, response and usage counter.
/// Implementations must attach a partial snapshot after making progress, set its
/// status to `Incomplete`, and leave final output unset on a turn-limit failure.
#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct RunError {
    #[source]
    pub error: Error,
    pub partial: Option<Box<crate::RunResult>>,
}

impl RunError {
    /// Attach accumulated state without copying a potentially large history.
    pub fn with_partial(error: Error, partial: crate::RunResult) -> Self {
        Self {
            error,
            partial: Some(Box::new(partial)),
        }
    }
}

impl From<Error> for RunError {
    fn from(error: Error) -> Self {
        Self {
            error,
            partial: None,
        }
    }
}
