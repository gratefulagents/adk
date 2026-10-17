use super::{
    AgentMeta, DEFAULT_MAX_TOOL_RESULT_BYTES, MAX_EVENTS, ProgressEvent, ProgressSnapshot,
};
use crate::{tracewriter::Span, tracing::TraceProcessor};
use chrono::{DateTime, SecondsFormat, Utc};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread::ThreadId,
    time::SystemTime,
};

pub(super) struct Notification {
    processor: Arc<dyn TraceProcessor>,
    span: Span,
    start: bool,
}

pub(super) struct State {
    pub snapshot: ProgressSnapshot,
    pub task_agents: BTreeMap<String, AgentMeta>,
    pub last_assistant_text: String,
    pub last_assistant_thinking: String,
    pub session_tool_call_count: i32,
    pub session_cost_usd: f64,
    pub pending_events: Vec<ProgressEvent>,
    pub max_tool_result_bytes: i64,
    pub task_id: String,
    pub parent: Option<ProgressTracker>,
    pub processor: Option<Arc<dyn TraceProcessor>>,
    pub root_span_id: String,
    pub subagent_spans: BTreeMap<String, Span>,
    notifications: VecDeque<Notification>,
    dispatching: Option<ThreadId>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            snapshot: ProgressSnapshot::default(),
            task_agents: BTreeMap::new(),
            last_assistant_text: String::new(),
            last_assistant_thinking: String::new(),
            session_tool_call_count: 0,
            session_cost_usd: 0.0,
            pending_events: Vec::new(),
            max_tool_result_bytes: DEFAULT_MAX_TOOL_RESULT_BYTES,
            task_id: String::new(),
            parent: None,
            processor: None,
            root_span_id: String::new(),
            subagent_spans: BTreeMap::new(),
            notifications: VecDeque::new(),
            dispatching: None,
        }
    }
}

impl State {
    pub fn event(&mut self, event_type: &str, summary: String, pending: bool) {
        let event = ProgressEvent {
            timestamp: serde_json::from_value(
                DateTime::<Utc>::from(SystemTime::now())
                    .to_rfc3339_opts(SecondsFormat::Nanos, true)
                    .into(),
            )
            .expect("system time is an RFC3339 timestamp"),
            event_type: event_type.into(),
            summary,
        };
        if pending {
            self.pending_events.push(event.clone());
        }
        self.snapshot.events.push(event);
        if self.snapshot.events.len() > MAX_EVENTS {
            self.snapshot.events.remove(0);
        }
    }

    pub fn notify(&mut self, span: Span, start: bool) {
        if let Some(processor) = &self.processor {
            self.notifications.push_back(Notification {
                processor: processor.clone(),
                span,
                start,
            });
        }
    }

    pub fn instant_span(&mut self, name: &str, data: crate::tracewriter::SpanData) {
        if self.processor.is_some() {
            let mut span = Span::new(name, &self.root_span_id, Some(data));
            self.notify(span.clone(), true);
            span.finish();
            self.notify(span, false);
        }
    }

    pub fn infer_step(&mut self, tool_name: &str, command: &str) {
        let step = self.snapshot.current_step.as_str();
        let inferred = match tool_name {
            "LSP" if step == "starting" || step.is_empty() => "exploring",
            "Edit" | "Write" if step != "committing" && step != "reviewing" => "implementing",
            "Bash" if command.contains("git commit") || command.contains("git add") => "committing",
            "Bash"
                if command.contains("git diff")
                    && step != "implementing"
                    && step != "committing" =>
            {
                "reviewing"
            }
            _ => step,
        };
        if inferred != step {
            self.snapshot.current_step = inferred.into();
            self.event(
                "step_change",
                format!("Step: {} (via {tool_name})", self.snapshot.current_step),
                false,
            );
        }
    }
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    callbacks_drained: Condvar,
}

#[derive(Clone, Default)]
pub struct ProgressTracker(Arc<Shared>);

impl ProgressTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().expect("progress tracker poisoned")
    }

    pub fn child(&self, task_id: impl Into<String>) -> Self {
        let task_id = task_id.into();
        let parent = self.lock();
        Self(Arc::new(Shared {
            state: Mutex::new(State {
                snapshot: ProgressSnapshot {
                    session_number: parent.snapshot.session_number,
                    ..Default::default()
                },
                task_id,
                parent: Some(self.clone()),
                max_tool_result_bytes: parent.max_tool_result_bytes,
                processor: parent.processor.clone(),
                root_span_id: parent.root_span_id.clone(),
                ..Default::default()
            }),
            callbacks_drained: Condvar::new(),
        }))
    }

    pub fn with_max_tool_result_bytes(self, bytes: i64) -> Self {
        self.lock().max_tool_result_bytes = bytes;
        self
    }

    pub fn with_tracing_processor(self, processor: Arc<dyn TraceProcessor>) -> Self {
        self.set_tracing_processor(Some(processor));
        self
    }

    pub fn set_tracing_processor(&self, processor: Option<Arc<dyn TraceProcessor>>) {
        let previous = std::mem::replace(&mut self.lock().processor, processor);
        drop(previous);
    }

    pub fn set_root_span_id(&self, id: &str) {
        self.lock().root_span_id = id.into();
    }

    pub fn root_span_id(&self) -> String {
        self.lock().root_span_id.clone()
    }
    pub fn max_tool_result_bytes(&self) -> i64 {
        self.lock().max_tool_result_bytes
    }
    pub fn task_id(&self) -> String {
        self.lock().task_id.clone()
    }
    pub fn session_number(&self) -> i32 {
        self.lock().snapshot.session_number
    }
    pub fn current_step(&self) -> String {
        self.lock().snapshot.current_step.clone()
    }
    pub fn last_assistant_text(&self) -> String {
        self.lock().last_assistant_text.clone()
    }
    pub fn last_assistant_thinking(&self) -> String {
        self.lock().last_assistant_thinking.clone()
    }
    pub fn session_tool_call_count(&self) -> i32 {
        self.lock().session_tool_call_count
    }
    pub fn session_cost_usd(&self) -> f64 {
        self.lock().session_cost_usd
    }
    pub fn agent_meta(&self, task_id: &str) -> Option<AgentMeta> {
        self.lock().task_agents.get(task_id).cloned()
    }

    pub fn set_session(&self, number: i32, step: &str) {
        let mut state = self.lock();
        state.snapshot.session_number = number;
        state.snapshot.current_step = step.into();
        state.session_tool_call_count = 0;
        state.session_cost_usd = 0.0;
        state.event(
            "session_start",
            format!("Session {number} started: {step}"),
            true,
        );
    }

    pub fn set_step(&self, step: &str) {
        let mut state = self.lock();
        if state.snapshot.current_step != step {
            state.snapshot.current_step = step.into();
            state.event("step_change", format!("Step: {step}"), false);
        }
    }

    pub fn write_result(&self, status: &str, pr_url: &str, error: &str, failed_step: &str) {
        let mut parts = Vec::new();
        if !status.is_empty() {
            parts.push(format!("status={status}"));
        }
        if !pr_url.is_empty() {
            parts.push(format!("pr={pr_url}"));
        }
        if !failed_step.is_empty() {
            parts.push(format!("failed_step={failed_step}"));
        }
        if !error.is_empty() {
            parts.push(format!("error={}", truncate(error, 120)));
        }
        let summary = if parts.is_empty() {
            "run finished".into()
        } else {
            parts.join(" ")
        };
        let mut state = self.lock();
        state.snapshot.last_activity = summary.clone();
        state.event("run_result", summary, false);
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        self.lock().snapshot.clone()
    }

    pub fn drain_pending_events(&self) -> Vec<ProgressEvent> {
        std::mem::take(&mut self.lock().pending_events)
    }

    pub(super) fn dispatch(&self) {
        {
            let mut state = self.lock();
            let thread = std::thread::current().id();
            while let Some(owner) = state.dispatching {
                // Same-thread callback reentry queues behind the current callback.
                // Other callers must wait for delivery before returning, so an
                // immediate host flush cannot overtake their completion spans.
                if owner == thread {
                    return;
                }
                state = self
                    .0
                    .callbacks_drained
                    .wait(state)
                    .expect("progress tracker poisoned");
            }
            state.dispatching = Some(thread);
        }
        // A single drainer preserves start/end order even when a processor reenters.
        loop {
            let notification = {
                let mut state = self.lock();
                match state.notifications.pop_front() {
                    Some(notification) => notification,
                    None => {
                        state.dispatching = None;
                        self.0.callbacks_drained.notify_all();
                        return;
                    }
                }
            };
            if notification.start {
                notification.processor.span_start(&notification.span);
            } else {
                notification.processor.span_end(&notification.span);
            }
        }
    }
}

pub(super) fn truncate(text: &str, limit: usize) -> String {
    super::helpers::truncate(text, limit as i64)
}

#[cfg(test)]
mod tests {
    use super::ProgressTracker;

    #[test]
    fn signed_counters_wrap_and_completed_spans_are_removed() {
        let tracker = ProgressTracker::new();
        {
            let mut state = tracker.lock();
            state.snapshot.tool_call_count = i32::MAX;
            state.session_tool_call_count = i32::MAX;
            state.snapshot.agent_count = i32::MAX;
            state.snapshot.api_retries = i32::MAX;
        }
        tracker.record_tool_use("Read", "", "", 0, "", "", "");
        tracker.record_subagent_started("a", "", "", "", "", "", "");
        tracker.record_api_retry("", 0, 0, 0);
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.tool_call_count, i32::MIN);
        assert_eq!(tracker.session_tool_call_count(), i32::MIN);
        assert_eq!(snapshot.agent_count, i32::MIN);
        assert_eq!(snapshot.api_retries, i32::MIN);

        tracker.set_tracing_processor(Some(std::sync::Arc::new(
            crate::tracing::CompositeTraceProcessor::default(),
        )));
        tracker.record_subagent_started("a", "", "", "", "", "", "");
        assert_eq!(tracker.lock().subagent_spans.len(), 1);
        tracker.record_subagent_completed(
            "a",
            "completed",
            "",
            0.0,
            0,
            &Default::default(),
            "",
            &[],
            &[],
        );
        assert!(tracker.lock().subagent_spans.is_empty());
    }
}
