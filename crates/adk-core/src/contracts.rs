use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    ApprovalRequest, Error, ErrorCategory, ModelEvent, ModelRequest, ModelResponse, RunError,
    RunEvent, RunRequest, RunResult, ToolCall, ToolDefinition, ToolOutput, ToolPolicy,
};

/// Object-safe asynchronous operation; borrowing does not require `'static`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Read-only, runtime-neutral cancellation signal supplied by an owner.
/// Once cancelled, it stays cancelled. Waiting must be race-free, cancel-safe,
/// and immediately ready for late subscribers. It never grants cancellation authority.
pub trait Cancellation: Send + Sync {
    fn is_cancelled(&self) -> bool;
    fn cancelled(&self) -> BoxFuture<'_, ()>;
}

/// Immutable operation boundary. Deadlines are monotonic and process-local;
/// adapters must race pending I/O against cancellation and deadline expiry.
/// `check_active` alone cannot interrupt an in-flight operation.
#[derive(Clone)]
pub struct Context {
    pub run_id: String,
    pub cancellation: Arc<dyn Cancellation>,
    pub deadline: Option<Instant>,
}

impl Context {
    /// Check before starting new work; cancellation takes precedence over timeout.
    pub fn check_active(&self) -> Result<(), Error> {
        if self.cancellation.is_cancelled() {
            Err(Error::new(ErrorCategory::Cancelled, "operation cancelled"))
        } else if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            Err(Error::new(
                ErrorCategory::DeadlineExceeded,
                "deadline exceeded",
            ))
        } else {
            Ok(())
        }
    }
}

/// Host-trusted tool execution boundary; do not populate from model arguments.
/// The executor must enforce policy and approval before calling `Tool::execute`.
pub struct ToolContext {
    pub operation: Context,
    pub work_dir: PathBuf,
    pub policy: ToolPolicy,
    pub idempotency_key: Option<String>,
}

/// Pluggable agent executor. Core supplies no default runner or pretend result.
pub trait Agent: Send + Sync {
    fn name(&self) -> &str;
    fn run<'a>(
        &'a self,
        context: &'a Context,
        request: RunRequest,
        host: &'a dyn Host,
    ) -> BoxFuture<'a, Result<RunResult, RunError>>;
}

/// Provider retry guidance; the runner owns precedence, bounds and cancellation.
#[derive(Debug, Clone)]
pub struct ModelRetryAdvice {
    pub should_retry: bool,
    pub retry_after: Duration,
    pub reason: String,
}

/// Provider-neutral completion boundary. Streaming is a separate capability;
/// implementations must not simulate it by silently buffering a complete call.
pub trait Model: Send + Sync {
    fn retry_advice(&self, _error: &Error) -> Option<ModelRetryAdvice> {
        None
    }
    fn provider(&self) -> &str;
    fn complete<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>>;
}

/// Opt-in genuine streaming capability; no unsupported default implementation.
pub trait StreamingModel: Model {
    fn stream<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>>;
}

/// Pull-based stream with backpressure, independent of any executor/Stream crate.
/// Dropping the stream must stop its owned producer. After error or clean EOF,
/// subsequent calls must return `Ok(None)`; clean EOF requires a Complete event.
pub trait ModelStream: Send {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>>;
}

/// Executable tool. Model-visible failures use `ToolOutput::is_error`;
/// infrastructure, cancellation and authorization failures use `Err`.
pub trait Tool: Send + Sync {
    fn definition(&self) -> &ToolDefinition;
    /// Host-authored lifecycle/control capability, never inferred from a tool name.
    /// Exempts mutation-only approval policy, not authorization or tool-owned approval.
    fn is_control_flow(&self) -> bool {
        false
    }
    /// Optional tool default; explicit host policy takes precedence.
    fn timeout(&self) -> Option<Duration> {
        None
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>>;
}

/// An approval never authorizes future calls or bypasses access restrictions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approve,
    Deny,
    /// Pause execution and return the unresolved call in the run snapshot.
    Defer,
}

/// Application boundary for ordered event delivery and exact-call approval.
/// Awaiting `emit` applies backpressure; errors must be propagated by the runner.
/// Persistence, sessions, authentication and sandboxing belong to host adapters.
pub trait Host: Send + Sync {
    fn emit<'a>(
        &'a self,
        context: &'a Context,
        event: RunEvent,
    ) -> BoxFuture<'a, Result<(), Error>>;
    fn approve<'a>(
        &'a self,
        context: &'a Context,
        request: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>>;
}
