//! Provider-owned wire adapters; applications retain ownership of credential storage.
//!
//! Requests and streams borrow no global state. Dropping an in-flight operation drops
//! its HTTP future; no background tasks are spawned. See `docs/providers.md` for
//! the explicitly tested compatibility boundary.
pub mod auth;
pub mod client;
pub mod cost;
pub mod error;
pub mod oauth;
pub mod routing;
pub mod sse;
pub mod wire;

use adk_core::{Context, Error, ErrorCategory};
use std::future::Future;

pub(crate) fn invalid(message: &'static str) -> Error {
    Error::new(ErrorCategory::InvalidInput, message)
}

/// Race *each* blocking operation, including lock acquisition, with host cancellation.
pub(crate) async fn active<T>(
    context: &Context,
    operation: impl Future<Output = T>,
) -> Result<T, Error> {
    context.check_active()?;
    let deadline = async {
        match context.deadline {
            Some(at) => tokio::time::sleep_until(at.into()).await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        biased;
        _ = context.cancellation.cancelled() => Err(Error::new(ErrorCategory::Cancelled, "operation cancelled")),
        _ = deadline => Err(Error::new(ErrorCategory::DeadlineExceeded, "deadline exceeded")),
        result = operation => Ok(result),
    }
}
