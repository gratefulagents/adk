//! Explicit runtime observation adapter, not an implicit Runner default.
//!
//! Create one [`RunTrace`](crate::tracing_runtime::RunTrace) per run and install
//! the same [`observer`](crate::tracing_runtime::RunTrace::observer) Arc in
//! `RunnerConfig::hooks` and `RunnerConfig::generation_observer`. Keep the
//! owner alongside the run future: drop the future first, then finish/drop the
//! owner, including on errors, suspension and cancellation. Hooks have no reliable
//! run-end notification. Retaining a Runner does not retain the root after owner
//! completion; external TraceSession clones and manually created spans still do.
//!
//! Processors are synchronous and must not panic. Guarded function text is still
//! sensitive: processor capture/redaction policy remains the host's responsibility.
//! Generation snapshots precede tool guardrails and may contain rejected calls;
//! tool guardrails are not a blanket redaction policy for model snapshots.
//! Agent spans carry configured instructions; resolved per-attempt instructions
//! stay in generation snapshots. Neither provenance nor session metrics is inferred.
//! Concurrent/reentrant delivery is queued; the active caller drains callbacks
//! in queue order without holding a lock while invoking processors. Finish during
//! a callback is likewise queued and takes effect when that callback returns.

use crate::tracewriter::SpanData;
use crate::tracing::{SpanGuard, TraceSession};
use adk_core::{BoxFuture, Content, Context, Error};
use adk_runtime::tracing::{GenerationObserver, GenerationRecord};
use adk_runtime::{Observation, RunHooks};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

fn session_summary(
    result: &adk_core::RunResult,
    reason: &str,
) -> Result<crate::tracewriter::Session, &'static str> {
    let metrics = result
        .metrics
        .as_ref()
        .ok_or("session metrics unavailable")?;
    let count =
        |value| i64::try_from(value).map_err(|_| "session counter exceeds SDK signed range");
    Ok(crate::tracewriter::Session {
        model: metrics.model.clone().unwrap_or_default(),
        cost_usd: metrics.cost_usd,
        num_turns: i64::from(metrics.turns),
        duration_ms: count(metrics.elapsed_ms)?,
        input_tokens: count(result.usage.input_tokens)?,
        output_tokens: count(result.usage.output_tokens)?,
        cache_read_input_tokens: count(result.usage.cache_read_tokens)?,
        cache_creation_input_tokens: count(result.usage.cache_creation_tokens)?,
        stop_reason: reason.into(),
    })
}

/// Per-run cleanup authority, independent of observer Arcs retained by a Runner.
#[must_use = "keep the owner until the run future has completed or been dropped"]
pub struct RunTrace {
    observer: Arc<RuntimeTracing>,
}
impl RunTrace {
    /// Transfer one shared root owner; this does not create a second trace.
    pub fn new(session: TraceSession) -> Self {
        Self {
            observer: Arc::new(RuntimeTracing {
                queue: Mutex::new(Queue {
                    state: Some(State {
                        session: Some(session),
                        agent: None,
                        tools: HashMap::new(),
                        compaction: None,
                        generations: HashMap::new(),
                    }),
                    events: VecDeque::new(),
                }),
            }),
        }
    }
    /// Own a run future and enforce cleanup order on success, failure or Drop.
    /// The future is dropped before the trace owner, even when it never completes.
    pub fn run<F: std::future::Future>(self, future: F) -> TracedRun<F> {
        TracedRun {
            future: Some(Box::pin(future)),
            owner: Some(self),
        }
    }
    /// Emit a point session span from authoritative cumulative runner metrics.
    /// Pauses are boundaries, not final completion. Dropping a pending future
    /// emits no fabricated session summary. Missing legacy metrics or counters
    /// outside the SDK signed range are reported to the processor's error sink.
    pub fn run_session<F>(
        self,
        future: F,
    ) -> TracedRun<
        impl std::future::Future<Output = Result<adk_runtime::RunOutcome, adk_core::RunError>>,
    >
    where
        F: std::future::Future<Output = Result<adk_runtime::RunOutcome, adk_core::RunError>>,
    {
        let observer = self.observer.clone();
        self.run(async move {
            let outcome = future.await;
            let result = match &outcome {
                Ok(outcome) => Some(&outcome.result),
                Err(error) => error.partial.as_deref(),
            };
            if let Some(result) = result {
                let reason = match &outcome {
                    Ok(_) => serde_json::to_value(result.status).expect("serializable status"),
                    Err(error) => serde_json::to_value(error.error.info.category)
                        .expect("serializable category"),
                };
                observer.dispatch(Event::SessionComplete(session_summary(
                    result,
                    reason.as_str().expect("string enum"),
                )));
            } else {
                observer.dispatch(Event::SessionComplete(Err(
                    "session metrics unavailable: no runner result",
                )));
            }
            outcome
        })
    }
    pub fn observer(&self) -> Arc<RuntimeTracing> {
        self.observer.clone()
    }
    /// End remaining spans and release the root, without flushing host processors.
    /// Drop the run future first to preserve its final generation status. Further
    /// callbacks are ignored; a new run requires a new owner/adapter.
    pub fn finish(self) {
        drop(self);
    }
}
impl Drop for RunTrace {
    fn drop(&mut self) {
        self.observer.dispatch(Event::Finish);
    }
}

/// A run future with owned, ordered trace cleanup. Dropping it cancels the inner
/// future before completing the trace. It never spawns or detaches work.
#[must_use = "futures do nothing unless polled"]
pub struct TracedRun<F> {
    future: Option<std::pin::Pin<Box<F>>>,
    owner: Option<RunTrace>,
}
impl<F> TracedRun<F> {
    fn finish(&mut self) {
        self.future.take();
        self.owner.take();
    }
}
impl<F: std::future::Future> std::future::Future for TracedRun<F> {
    type Output = F::Output;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        let result = this
            .future
            .as_mut()
            .expect("traced run polled after completion")
            .as_mut()
            .poll(cx);
        if result.is_ready() {
            this.finish();
        }
        result
    }
}
impl<F> Drop for TracedRun<F> {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Install the same Arc in both RunnerConfig observer fields for exactly one run.
/// Repeated active agent/tool/generation starts are ignored. Agent lifetimes span
/// repeated model attempts and end at handoff, AgentEnded or owner completion.
/// Handoff is a point observation, not a measurement of the handoff operation.
/// Compaction has counts only on success; failures/drops have no fabricated data.
/// Function output joins guarded text/reasoning blocks with newlines, omits media,
/// and precedes native output caps and untrusted wrapping.
pub struct RuntimeTracing {
    queue: Mutex<Queue>,
}
struct Queue {
    state: Option<State>,
    events: VecDeque<Event>,
}
enum Event {
    SessionComplete(Result<crate::tracewriter::Session, &'static str>),
    Observation(Observation),
    Start(Context, Box<GenerationRecord>),
    End(Context, Box<GenerationRecord>),
    Finish,
}
struct State {
    session: Option<TraceSession>,
    agent: Option<(String, SpanGuard)>,
    tools: HashMap<String, SpanGuard>,
    compaction: Option<(u64, SpanGuard)>,
    generations: HashMap<String, Arc<dyn GenerationObserver>>,
}
impl RuntimeTracing {
    fn dispatch(&self, event: Event) {
        let mut state = {
            let mut queue = self.queue.lock().expect("runtime trace queue poisoned");
            queue.events.push_back(event);
            let Some(state) = queue.state.take() else {
                return;
            };
            state
        };
        // The drainer owns state outside the lock; reentrant callbacks queue work.
        loop {
            let event = {
                let mut queue = self.queue.lock().expect("runtime trace queue poisoned");
                match queue.events.pop_front() {
                    Some(event) => event,
                    None => {
                        queue.state = Some(state);
                        return;
                    }
                }
            };
            state.apply(event);
        }
    }
}
impl RunHooks for RuntimeTracing {
    fn durable_observer(&self) -> bool {
        true
    }
    fn observe<'a>(
        &'a self,
        _: &'a Context,
        observation: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.dispatch(Event::Observation(observation));
            Ok(())
        })
    }
}
impl GenerationObserver for RuntimeTracing {
    fn start(&self, context: &Context, record: &GenerationRecord) {
        self.dispatch(Event::Start(context.clone(), Box::new(record.clone())));
    }
    fn end(&self, context: &Context, record: &GenerationRecord) {
        self.dispatch(Event::End(context.clone(), Box::new(record.clone())));
    }
}
impl State {
    fn span(&self, name: &str, data: Option<SpanData>) -> SpanGuard {
        match &self.agent {
            Some((_, agent)) => agent.child(name, data),
            None => self.session.as_ref().expect("open trace").span(name, data),
        }
    }
    fn apply(&mut self, event: Event) {
        if self.session.is_none() {
            return;
        }
        match event {
            Event::SessionComplete(summary) => {
                let session = self.session.as_ref().expect("open trace");
                match summary {
                    Ok(summary) => {
                        session
                            .span("session", Some(SpanData::Session(summary)))
                            .finish();
                    }
                    Err(error) => session.report_error(error),
                }
            }
            Event::Finish => {
                self.generations.clear();
                for (_, mut span) in self.tools.drain() {
                    if let Some(SpanData::Function {
                        output, is_error, ..
                    }) = span.data_mut()
                    {
                        *is_error = true;
                        *output = "tool execution interrupted before an observed result".into();
                    }
                    span.finish();
                }
                self.compaction.take();
                self.agent.take();
                self.session.take();
            }
            Event::Start(context, record) => {
                if self.generations.contains_key(&record.id) {
                    return;
                }
                let observer = match &self.agent {
                    Some((_, agent)) => agent.generation_observer(),
                    None => self
                        .session
                        .as_ref()
                        .expect("open trace")
                        .generation_observer(),
                };
                observer.start(&context, &record);
                self.generations.insert(record.id.clone(), observer);
            }
            Event::End(context, record) => {
                if let Some(observer) = self.generations.remove(&record.id) {
                    observer.end(&context, &record);
                }
            }
            Event::Observation(observation) => match observation {
                Observation::AgentStarted {
                    agent,
                    instructions,
                } => {
                    if self.agent.as_ref().is_some_and(|(name, _)| name == &agent) {
                        return;
                    }
                    self.agent.take();
                    let span = self.session.as_ref().expect("open trace").span(
                        "agent",
                        Some(SpanData::Agent {
                            agent_name: agent.clone(),
                            instructions,
                        }),
                    );
                    self.agent = Some((agent, span));
                }
                Observation::AgentEnded { agent, .. } => {
                    if self.agent.as_ref().is_some_and(|(name, _)| name == &agent) {
                        self.agent.take();
                    }
                }
                Observation::ToolStarted { call, .. } => {
                    if !self.tools.contains_key(&call.id) {
                        let span = self.span(
                            "function",
                            Some(SpanData::Function {
                                tool_name: call.name,
                                input: call.arguments.to_string(),
                                output: String::new(),
                                is_error: false,
                            }),
                        );
                        self.tools.insert(call.id, span);
                    }
                }
                Observation::RawToolOutput { call, output } => {
                    if let Some(mut span) = self.tools.remove(&call.id) {
                        if let Some(SpanData::Function {
                            output: text,
                            is_error,
                            ..
                        }) = span.data_mut()
                        {
                            *text = output
                                .content
                                .iter()
                                .filter_map(|part| match part {
                                    Content::Text { text } | Content::Reasoning { text, .. } => {
                                        Some(text.as_str())
                                    }
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            *is_error = output.is_error;
                        }
                        span.finish();
                    }
                }
                Observation::Handoff { from, to } => {
                    self.span(
                        "handoff",
                        Some(SpanData::Handoff {
                            from_agent: from,
                            to_agent: to,
                        }),
                    )
                    .finish();
                    self.agent.take();
                }
                Observation::CompactionStarted { context_tokens, .. } => {
                    // Native no-op local compaction can return without a terminal
                    // observation. A new attempt must not reuse its counts/parent.
                    self.compaction.take();
                    self.compaction = Some((context_tokens, self.span("compaction", None)));
                }
                Observation::Compacted { context_tokens, .. } => {
                    if let Some((before, mut span)) = self.compaction.take() {
                        if let (Ok(tokens_before), Ok(tokens_after)) =
                            (i64::try_from(before), i64::try_from(context_tokens))
                        {
                            *span.data_mut() = Some(SpanData::Compaction {
                                tokens_before,
                                tokens_after,
                            });
                        }
                        span.finish();
                    }
                }
                Observation::CompactionFailed { .. } => {
                    self.compaction.take();
                }
                _ => {}
            },
        }
    }
}
