//! SDK progress state and structural records; not a replacement for native observability.
//!
//! Clones share state. Children own their activity and forward only usage and tool
//! counts to ancestors. Processors remain host-owned and are never flushed here.
//! As required by `TraceProcessor`, callbacks must not panic: a panic interrupts
//! the notification drain and subsequent delivery is not supported.

pub mod runtime;

mod helpers;
mod record;
mod state;
mod types;

pub use helpers::*;
pub use state::ProgressTracker;
pub use types::{
    AgentMeta, HookEvent, HookResult, ModelUsageEntry, ProgressEvent, ProgressSnapshot,
};

pub type RunProgress = ProgressTracker;
pub const MAX_EVENTS: usize = 20;
pub const DEFAULT_MAX_TOOL_RESULT_BYTES: i64 = 32 * 1024;
