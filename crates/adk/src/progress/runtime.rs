//! Opt-in runtime bridge. Native cumulative observability is independent.
//!
//! The bridge owns accepted assistant content by default. Set `content` to
//! `External` when another adapter already records it in this tracker. Structural
//! session/compaction spans default to external ownership. In tracker-owned mode,
//! wrap only the tracing hook with `tracing_hooks` (not policy hooks), and finish
//! `RunTrace` without its session summary. Root progress sessions are explicit;
//! agent handoffs never start or finish a session.

use super::ProgressTracker;
use adk_core::{BoxFuture, Content, Context, Error, Role, RunItem, RunResult};
use adk_runtime::{Observation, RunHooks, progress::*};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::Instant,
};

pub type UsdPrice =
    dyn Fn(ProgressFrame<'_>, ProgressUsage<'_>) -> Result<f64, String> + Send + Sync;
pub type ModelLabel = dyn Fn(ProgressFrame<'_>, ProgressUsage<'_>) -> String + Send + Sync;
pub type Diagnostic = dyn Fn(&str) + Send + Sync;

#[derive(Clone, Default)]
pub enum UsdPolicy {
    /// Token counts are recorded, but no USD value is inferred from host units.
    #[default]
    Unpriced,
    HostUnitsAreUsd,
    Custom(Arc<UsdPrice>),
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum ContentOwnership {
    #[default]
    Tracker,
    External,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum SpanOwnership {
    #[default]
    External,
    Tracker,
}

#[derive(Clone, Default)]
pub struct ProgressOptions {
    pub usd: UsdPolicy,
    pub content: ContentOwnership,
    pub spans: SpanOwnership,
    pub model_label: Option<Arc<ModelLabel>>,
    pub diagnostic: Option<Arc<Diagnostic>>,
}

pub struct RuntimeProgressAdapter {
    tracker: ProgressTracker,
    options: ProgressOptions,
    children: Mutex<HashMap<String, Arc<RuntimeProgressAdapter>>>,
    compaction_before: Mutex<Option<u64>>,
    turn: AtomicU32,
    diagnostics: Arc<Mutex<Vec<String>>>,
}
impl RuntimeProgressAdapter {
    pub fn new(tracker: ProgressTracker, options: ProgressOptions) -> Self {
        Self {
            tracker,
            options,
            children: Mutex::new(HashMap::new()),
            compaction_before: Mutex::new(None),
            turn: AtomicU32::new(0),
            diagnostics: Arc::new(Mutex::new(Vec::new())),
        }
    }
    pub fn tracker(&self) -> &ProgressTracker {
        &self.tracker
    }
    pub fn drain_diagnostics(&self) -> Vec<String> {
        std::mem::take(&mut *self.diagnostics.lock().unwrap())
    }
    fn diagnostic(&self, message: &str) {
        self.diagnostics.lock().unwrap().push(message.into());
        if let Some(report) = &self.options.diagnostic {
            report(message);
        }
    }
    fn signed(&self, value: u64) -> Option<i64> {
        match i64::try_from(value) {
            Ok(value) => Some(value),
            Err(_) => {
                self.diagnostic("progress counter exceeds SDK signed range");
                None
            }
        }
    }
    fn usage_dto(&self, usage: &adk_core::Usage) -> Option<adk_codec::dto::Usage> {
        Some(adk_codec::dto::Usage {
            requests: self.signed(usage.requests)?,
            input_tokens: self.signed(usage.input_tokens)?,
            output_tokens: self.signed(usage.output_tokens)?,
            cache_read_tokens: self.signed(usage.cache_read_tokens)?,
            cache_create_tokens: self.signed(usage.cache_creation_tokens)?,
        })
    }
    /// Explicit facade scope; not inferred from AgentStarted/AgentEnded or handoffs.
    pub fn session(self: &Arc<Self>, number: i32, step: &str) -> ProgressSession {
        self.tracker.set_session(number, step);
        ProgressSession {
            adapter: self.clone(),
            started: Instant::now(),
        }
    }
    /// Filter only the tracing hook when this adapter owns compaction spans.
    pub fn tracing_hooks(&self, hooks: Arc<dyn RunHooks>) -> Arc<dyn RunHooks> {
        if self.options.spans == SpanOwnership::Tracker {
            Arc::new(TracingFilter(hooks))
        } else {
            hooks
        }
    }
}

impl RuntimeProgress for RuntimeProgressAdapter {
    fn usage(&self, _cx: &Context, frame: ProgressFrame<'_>, call: ProgressUsage<'_>) {
        if call.origin == UsageOrigin::FailedStreamRetained {
            return;
        }
        let used = call.usage;
        if call.origin != UsageOrigin::Accepted
            && used.input_tokens == 0
            && used.output_tokens == 0
            && used.cache_read_tokens == 0
            && used.cache_creation_tokens == 0
        {
            return;
        }
        let Some(usage) = self.usage_dto(used) else {
            return;
        };
        let price = match &self.options.usd {
            UsdPolicy::Unpriced => Ok(0.0),
            UsdPolicy::HostUnitsAreUsd => {
                if !call.host_cost_known {
                    self.diagnostic("USD price coverage is uncertain");
                }
                call.host_cost
                    .ok_or_else(|| "host USD cost is absent".into())
            }
            UsdPolicy::Custom(price) => price(frame, call),
        };
        let cost = match price {
            Ok(cost) if cost.is_finite() && cost >= 0.0 => cost,
            Ok(_) => {
                self.diagnostic("invalid USD price");
                0.0
            }
            Err(error) => {
                self.diagnostic(&error);
                0.0
            }
        };
        let label = self
            .options
            .model_label
            .as_ref()
            .map(|label| label(frame, call));
        let model = label
            .as_deref()
            .unwrap_or(if call.origin == UsageOrigin::Accepted {
                frame.agent_model
            } else {
                call.request_model
            });
        self.tracker.record_llm_usage(model, cost, &usage);
    }
    fn observe(&self, _cx: &Context, frame: ProgressFrame<'_>, event: ProgressObservation<'_>) {
        self.turn.fetch_max(frame.turn, Ordering::Relaxed);
        let event = match event {
            ProgressObservation::Retry {
                error,
                attempt,
                max_retries,
                delay,
            } => {
                let converted = (
                    i64::try_from(delay.as_millis()),
                    i32::try_from(attempt),
                    i32::try_from(max_retries),
                );
                if let (Ok(delay), Ok(attempt), Ok(max_retries)) = converted {
                    self.tracker
                        .record_api_retry(&error.message, delay, attempt, max_retries);
                } else {
                    self.diagnostic("retry counter exceeds SDK signed range");
                }
                return;
            }
            ProgressObservation::Lifecycle(event) => event,
        };
        match event {
            Observation::ModelAccepted { response, .. }
                if self.options.content == ContentOwnership::Tracker =>
            {
                let mut text = String::new();
                let mut thinking = String::new();
                for item in &response.items {
                    match item {
                        RunItem::Message { message } | RunItem::PhasedMessage { message, .. }
                            if message.role == Role::Assistant =>
                        {
                            for content in &message.content {
                                match content {
                                    Content::Text { text: value } => text.push_str(value),
                                    Content::Reasoning { text: value, .. } => {
                                        thinking.push_str(value)
                                    }
                                    _ => {}
                                }
                            }
                        }
                        RunItem::Reasoning { reasoning } => thinking.push_str(&reasoning.text),
                        _ => {}
                    }
                }
                if !text.is_empty() {
                    self.tracker.record_assistant_text(&text);
                }
                if !thinking.is_empty() {
                    self.tracker.record_assistant_thinking(&thinking);
                }
            }
            Observation::ToolStarted { call, .. } => self.tracker.record_tool_use(
                &call.name,
                &call.arguments.to_string(),
                &call.id,
                i64::from(frame.turn),
                &call.arguments.to_string(),
                frame.agent_name,
                frame.task_id.unwrap_or(""),
            ),
            Observation::RawToolOutput { call, output } => {
                let text: String = output
                    .content
                    .iter()
                    .filter_map(|content| match content {
                        Content::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                self.tracker.record_tool_result(
                    &call.id,
                    &call.name,
                    &text,
                    output.is_error,
                    0,
                    frame.agent_name,
                    frame.task_id.unwrap_or(""),
                );
            }
            Observation::CompactionStarted { context_tokens, .. } => {
                *self.compaction_before.lock().unwrap() = Some(*context_tokens);
            }
            Observation::CompactionFailed { .. } => {
                self.compaction_before.lock().unwrap().take();
            }
            Observation::Compacted { context_tokens, .. } => {
                let before = self.compaction_before.lock().unwrap().take();
                if self.options.spans == SpanOwnership::Tracker {
                    if let Some((before, after)) = before
                        .and_then(|before| self.signed(before))
                        .zip(self.signed(*context_tokens))
                    {
                        self.tracker
                            .record_compact_boundary(before, after, "runtime");
                    }
                }
            }
            _ => {}
        }
    }
    fn fork_child(&self, child: ProgressChild<'_>) -> Option<Arc<dyn RuntimeProgress>> {
        let adapter = self.restore_child(child)?;
        self.tracker.record_subagent_started(
            child.task_id,
            "",
            child.agent_name,
            child.agent_name,
            child.agent_model,
            "",
            "",
        );
        Some(adapter)
    }
    fn restore_child(&self, child: ProgressChild<'_>) -> Option<Arc<dyn RuntimeProgress>> {
        let mut children = self.children.lock().unwrap();
        let adapter = children.entry(child.task_id.into()).or_insert_with(|| {
            let mut adapter = Self::new(self.tracker.child(child.task_id), self.options.clone());
            adapter.diagnostics = self.diagnostics.clone();
            Arc::new(adapter)
        });
        Some(adapter.clone())
    }
    fn child_finished(&self, child: ProgressChild<'_>, end: ProgressChildEnd<'_>) {
        let Some(adapter) = self.children.lock().unwrap().remove(child.task_id) else {
            return;
        };
        let snapshot = adapter.tracker.snapshot();
        let (status, summary, reason) = match end {
            ProgressChildEnd::Finished(outcome) => {
                use adk_runtime::subagent::TaskStatus;
                let status = match outcome.status {
                    TaskStatus::Completed => "completed",
                    TaskStatus::Cancelled => "stopped",
                    _ => "failed",
                };
                (
                    status,
                    outcome.result.as_str(),
                    outcome.error.as_deref().unwrap_or(""),
                )
            }
            ProgressChildEnd::Failed(error) => ("failed", "", error.message.as_str()),
            ProgressChildEnd::Dropped => ("stopped", "", "dropped"),
        };
        self.tracker.record_subagent_completed(
            child.task_id,
            status,
            summary,
            snapshot.cost_usd,
            i64::from(adapter.turn.load(Ordering::Relaxed)),
            &adk_codec::dto::Usage {
                input_tokens: snapshot.input_tokens,
                output_tokens: snapshot.output_tokens,
                cache_read_tokens: snapshot.cache_read_input_tokens,
                cache_create_tokens: snapshot.cache_creation_input_tokens,
                ..Default::default()
            },
            reason,
            &[],
            &[],
        );
    }
}

/// Dropping this scope records no synthetic completion or usage.
#[must_use]
pub struct ProgressSession {
    adapter: Arc<RuntimeProgressAdapter>,
    started: Instant,
}
impl ProgressSession {
    pub fn finish(self, result: &RunResult, stop_reason: &str) {
        let turns = result
            .metrics
            .as_ref()
            .map_or(0, |metrics| i64::from(metrics.turns));
        if self.adapter.options.spans != SpanOwnership::Tracker {
            self.adapter
                .tracker
                .record_session_complete_event(self.adapter.tracker.session_cost_usd(), turns);
            return;
        }
        let Some(usage) = self.adapter.usage_dto(&result.usage) else {
            return;
        };
        let Ok(duration) = i64::try_from(self.started.elapsed().as_millis()) else {
            self.adapter
                .diagnostic("session duration exceeds SDK signed range");
            return;
        };
        let model = result
            .metrics
            .as_ref()
            .and_then(|metrics| metrics.model.as_deref())
            .unwrap_or("");
        self.adapter.tracker.record_session_complete(
            model,
            self.adapter.tracker.session_cost_usd(),
            turns,
            &usage,
            duration,
            stop_reason,
        );
    }
}
struct TracingFilter(Arc<dyn RunHooks>);
impl RunHooks for TracingFilter {
    fn durable_observer(&self) -> bool {
        self.0.durable_observer()
    }
    fn observe<'a>(
        &'a self,
        cx: &'a Context,
        event: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        if matches!(
            event,
            Observation::CompactionStarted { .. }
                | Observation::Compacted { .. }
                | Observation::CompactionFailed { .. }
        ) {
            Box::pin(async { Ok(()) })
        } else {
            self.0.observe(cx, event)
        }
    }
}
