//! Opt-in execution engine and explicitly owned Tokio lifecycle primitives.
//!
//! [`TaskGroup`] never detaches tasks: drop cancels and requests abortion;
//! [`TaskGroup::shutdown`] additionally joins every task before returning.
//! Abortion only takes effect when a task yields. Non-yielding work can prevent
//! shutdown; blocking processes/threads need their own owner and termination API.

pub mod compaction;
pub mod compat;
pub mod output;
pub mod runner;
pub use runner::*;

use std::future::Future;

use adk_core::{BoxFuture, Cancellation};
use tokio::task::{JoinError, JoinSet};

/// Hierarchical cancellation. Clones share authority; cancelling a child does
/// not cancel its parent, while cancelling a parent cancels all descendants.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(tokio_util::sync::CancellationToken);

impl CancellationToken {
    /// Create an independent, initially active cancellation boundary.
    pub fn new() -> Self {
        Self::default()
    }

    /// Derive a boundary whose authority is restricted to this subtree.
    pub fn child_token(&self) -> Self {
        Self(self.0.child_token())
    }

    /// Idempotently signal cancellation to all waiters and children.
    pub fn cancel(&self) {
        self.0.cancel();
    }

    /// Observe the latched cancellation state.
    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }

    /// Wait without missed wakeups, including after cancellation has occurred.
    pub async fn cancelled(&self) {
        self.0.cancelled().await;
    }
}

impl Cancellation for CancellationToken {
    fn is_cancelled(&self) -> bool {
        self.is_cancelled()
    }

    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(self.cancelled())
    }
}

/// Termination evidence for tasks still owned at shutdown. Panics are retained,
/// not silently swallowed; tasks already consumed via `join_next` are excluded.
#[derive(Debug, Default)]
pub struct ShutdownReport {
    pub completed: usize,
    pub cancelled: usize,
    pub panics: Vec<JoinError>,
}

/// Exclusive owner of asynchronous tasks and their cancellation subtree.
/// No join/abort handles escape. Use `join_next` to supervise work while active.
/// Tasks must not detach their own children or perform non-yielding blocking work.
pub struct TaskGroup {
    cancellation: CancellationToken,
    tasks: JoinSet<()>,
}

impl TaskGroup {
    /// Own a new child scope without acquiring authority over the parent.
    pub fn new(parent: &CancellationToken) -> Self {
        Self {
            cancellation: parent.child_token(),
            tasks: JoinSet::new(),
        }
    }

    /// Observe or cooperatively cancel this scope, but not its parent.
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// Start a task with a child cancellation boundary. Must be called within a
    /// Tokio runtime. A cancelled scope remains cancelled for newly spawned work.
    pub fn spawn<F, Fut>(&mut self, make_future: F)
    where
        F: FnOnce(CancellationToken) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.tasks
            .spawn(make_future(self.cancellation.child_token()));
    }

    /// Join the next finished task; cancellation-safe if this wait is dropped.
    pub async fn join_next(&mut self) -> Option<Result<(), JoinError>> {
        self.tasks.join_next().await
    }

    /// Cancel, abort and join all remaining tasks, retaining panic evidence.
    /// This is prompt teardown, not a grace period for async cleanup. Dropping
    /// this shutdown future still cancels/aborts through the owner's destructor.
    pub async fn shutdown(mut self) -> ShutdownReport {
        self.cancellation.cancel();
        self.tasks.abort_all();
        let mut report = ShutdownReport::default();
        while let Some(result) = self.tasks.join_next().await {
            match result {
                Ok(()) => report.completed += 1,
                Err(error) if error.is_cancelled() => report.cancelled += 1,
                Err(error) => report.panics.push(error),
            }
        }
        report
    }
}

impl Drop for TaskGroup {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.tasks.abort_all();
    }
}
