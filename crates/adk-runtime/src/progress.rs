//! Optional synchronous observations, independent of execution policy and tracing.

use crate::{Observation, subagent::ChildOutcome};
use adk_core::{Context, ErrorInfo, Usage};
use std::{sync::Arc, time::Duration};

#[derive(Debug, Clone, Copy)]
pub struct ProgressFrame<'a> {
    pub agent_name: &'a str,
    pub agent_model: &'a str,
    pub turn: u32,
    pub task_id: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageOrigin {
    Accepted,
    ProviderCompaction,
    SummaryCompaction,
    SteeringSuperseded,
    FailedStreamRetained,
}

#[derive(Debug, Clone, Copy)]
pub struct ProgressUsage<'a> {
    pub origin: UsageOrigin,
    pub request_model: &'a str,
    pub provider: &'a str,
    pub canonical_model: &'a str,
    pub usage: &'a Usage,
    /// Host-defined units, not necessarily USD.
    pub host_cost: Option<f64>,
    pub host_cost_known: bool,
}

pub enum ProgressObservation<'a> {
    Lifecycle(&'a Observation),
    Retry {
        error: &'a ErrorInfo,
        attempt: u32,
        max_retries: u32,
        delay: Duration,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct ProgressChild<'a> {
    pub task_id: &'a str,
    pub parent_task_id: Option<&'a str>,
    pub agent_name: &'a str,
    pub agent_model: &'a str,
}

pub enum ProgressChildEnd<'a> {
    Finished(&'a ChildOutcome),
    Failed(&'a ErrorInfo),
    /// Local future was dropped; no terminal response or usage is implied.
    Dropped,
}

/// Callbacks must not panic or detach work. They cannot reject execution.
pub trait RuntimeProgress: Send + Sync {
    fn observe(&self, _cx: &Context, _frame: ProgressFrame<'_>, _event: ProgressObservation<'_>) {}
    fn usage(&self, _cx: &Context, _frame: ProgressFrame<'_>, _call: ProgressUsage<'_>) {}
    fn fork_child(&self, _child: ProgressChild<'_>) -> Option<Arc<dyn RuntimeProgress>> {
        None
    }
    /// Rebuild an ancestor binding without emitting lifecycle events or replaying usage.
    /// A later `fork_child` for this task must reuse this binding, then report its start.
    /// The model is empty when the scheduler has no historical model identity.
    /// Returning `None` opts out of observations for descendants of this ancestor.
    fn restore_child(&self, _child: ProgressChild<'_>) -> Option<Arc<dyn RuntimeProgress>> {
        None
    }
    fn child_finished(&self, _child: ProgressChild<'_>, _end: ProgressChildEnd<'_>) {}
}
