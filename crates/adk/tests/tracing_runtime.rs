#![cfg(feature = "observability")]

use adk::core::*;
use adk::runtime::tracing::{GenerationObserver, GenerationRecord, GenerationStatus};
use adk::runtime::*;
use adk::tracewriter::{Span, SpanData, Trace};
use adk::tracing::{TraceProcessor, TraceSession};
use adk::tracing_runtime::{RunTrace, RuntimeTracing};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<(String, String)>>,
    starts: Mutex<Vec<Span>>,
    traces: Mutex<Vec<Trace>>,
}
impl TraceProcessor for Recorder {
    fn trace_start(&self, trace: &Trace) {
        self.events
            .lock()
            .unwrap()
            .push(("trace_start".into(), trace.id.clone()));
    }
    fn span_start(&self, span: &Span) {
        self.starts.lock().unwrap().push(span.clone());
        self.events
            .lock()
            .unwrap()
            .push(("start".into(), span.id.clone()));
    }
    fn span_end(&self, span: &Span) {
        self.events
            .lock()
            .unwrap()
            .push(("end".into(), span.id.clone()));
    }
    fn trace_end(&self, trace: &Trace) {
        self.traces.lock().unwrap().push(trace.clone());
        self.events
            .lock()
            .unwrap()
            .push(("trace_end".into(), trace.id.clone()));
    }
}
fn context() -> Context {
    Context {
        run_id: "run".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn setup() -> (Arc<Recorder>, RunTrace, Arc<RuntimeTracing>) {
    let sink = Arc::new(Recorder::default());
    let owner = RunTrace::new(TraceSession::new("run", sink.clone()));
    let observer = owner.observer();
    (sink, owner, observer)
}
fn assert_closed(sink: &Recorder) -> Trace {
    let traces = sink.traces.lock().unwrap();
    assert_eq!(traces.len(), 1);
    let trace = traces[0].clone();
    let events = sink.events.lock().unwrap();
    assert_eq!(events.first().unwrap().0, "trace_start");
    assert_eq!(events.last().unwrap().0, "trace_end");
    for span in &trace.spans {
        assert_eq!(
            events
                .iter()
                .filter(|(kind, id)| kind == "start" && id == &span.id)
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|(kind, id)| kind == "end" && id == &span.id)
                .count(),
            1
        );
        assert!(span.end_time >= span.start_time);
        assert!(trace.end_time >= span.end_time);
    }
    trace
}
struct HostSink;
impl Host for HostSink {
    fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { Ok(ApprovalDecision::Approve) })
    }
}
enum Step {
    Response(ModelResponse),
    Error,
    Pending,
}
struct Provider(Mutex<VecDeque<Step>>);
impl Model for Provider {
    fn provider(&self) -> &str {
        "fixture"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            let step = self.0.lock().unwrap().pop_front().unwrap();
            match step {
                Step::Response(response) => Ok(response),
                Step::Error => Err(Error::new(ErrorCategory::InvalidInput, "fixture failure")),
                Step::Pending => std::future::pending().await,
            }
        })
    }
}
fn agent(name: &str, steps: Vec<Step>) -> AgentConfig {
    AgentConfig::new(
        name,
        ModelBinding::complete("fixture", Arc::new(Provider(Mutex::new(steps.into())))),
    )
}
fn response(items: Vec<RunItem>) -> Step {
    Step::Response(ModelResponse {
        items,
        response_id: None,
        usage: Usage::default(),
        end_turn: None,
        metadata: Default::default(),
        raw: None,
    })
}
fn done() -> Step {
    response(vec![RunItem::Message {
        message: Message {
            role: Role::Assistant,
            content: vec![Content::Text {
                text: "done".into(),
            }],
        },
    }])
}
fn call() -> ToolCall {
    ToolCall {
        id: "call".into(),
        name: "tool".into(),
        arguments: json!({"value": "private-input"}),
    }
}
fn definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: name.into(),
        input_schema: serde_json::from_value(json!({"type":"object"})).unwrap(),
        read_only: true,
        requires_approval: false,
    }
}
struct MockTool {
    definition: ToolDefinition,
    pending: bool,
    fail: bool,
}
impl Tool for MockTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            if self.pending {
                std::future::pending::<()>().await;
            }
            if self.fail {
                return Err(Error::new(ErrorCategory::Tool, "tool failure"));
            }
            Ok(ToolOutput {
                content: vec![Content::Text {
                    text: "private-output".repeat(100),
                }],
                is_error: false,
                should_pause: false,
            })
        })
    }
}
struct Block;
impl Guardrail for Block {
    fn name(&self) -> &str {
        "block"
    }
    fn check<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
        _: GuardrailInput<'a>,
    ) -> BoxFuture<'a, Result<Option<GuardrailResult>, Error>> {
        Box::pin(async {
            Ok(Some(GuardrailResult {
                tripwire_triggered: true,
                ..Default::default()
            }))
        })
    }
}
fn config(observer: Arc<RuntimeTracing>) -> RunnerConfig {
    RunnerConfig {
        hooks: Some(observer.clone()),
        generation_observer: Some(observer),
        ..Default::default()
    }
}
fn request() -> RunRequest {
    RunRequest {
        input: vec![],
        policy: RunPolicy::default(),
    }
}

#[tokio::test]
async fn runner_tool_spans_use_guarded_output_before_caps_and_repeated_agent_starts() {
    for boundary in ["none", "input", "output", "tool_error"] {
        let (sink, owner, observer) = setup();
        let mut agent = agent(
            "agent",
            vec![response(vec![RunItem::ToolCall { call: call() }]), done()],
        );
        agent.tools.push(Arc::new(MockTool {
            definition: definition("tool"),
            pending: false,
            fail: boundary == "tool_error",
        }));
        let mut config = config(observer);
        config.output.max_bytes = Some(80);
        if boundary == "input" {
            config.tool_input_guardrails.push(Arc::new(Block));
        }
        if boundary == "output" {
            config.tool_output_guardrails.push(Arc::new(Block));
        }
        let runner = Runner::new(agent, config).unwrap();
        let outcome = runner
            .run(context(), request(), Arc::new(HostSink))
            .await
            .unwrap();
        owner.finish();
        let trace = assert_closed(&sink);
        if boundary == "output" {
            assert!(!format!("{trace:?}").contains("private-output"));
        }
        let agents: Vec<_> = trace
            .spans
            .iter()
            .filter(|span| span.name == "agent")
            .collect();
        assert_eq!(agents.len(), 1);
        if let Some(SpanData::Agent { instructions, .. }) = &agents[0].data {
            assert!(instructions.is_empty());
        }
        let functions: Vec<_> = trace
            .spans
            .iter()
            .filter(|span| span.name == "function")
            .collect();
        if boundary == "input" {
            assert!(functions.is_empty());
        } else {
            assert_eq!(functions.len(), 1);
            assert_eq!(functions[0].parent_id, agents[0].id);
            let Some(SpanData::Function {
                input,
                output,
                is_error,
                ..
            }) = &functions[0].data
            else {
                panic!()
            };
            assert!(input.contains("private-input"));
            assert_eq!(*is_error, boundary != "none");
            if boundary == "none" {
                assert_eq!(output, &"private-output".repeat(100));
                let result = outcome
                    .result
                    .history
                    .iter()
                    .find_map(|item| match item {
                        RunItem::ToolResult { output, .. } => Some(output),
                        _ => None,
                    })
                    .unwrap();
                assert!(format!("{:?}", result.content).len() < output.len());
            } else {
                assert!(!output.contains("private-output"));
            }
        }
        for generation in trace.spans.iter().filter(|span| span.name == "generation") {
            assert_eq!(generation.parent_id, agents[0].id);
        }
        let starts = sink.starts.lock().unwrap();
        for span in starts.iter().filter(|span| span.name == "function") {
            let Some(SpanData::Function { output, .. }) = &span.data else {
                panic!()
            };
            assert!(output.is_empty());
        }
        // Runner retains both observer Arcs, but explicit finish already closed root.
        drop(runner);
        assert_eq!(sink.traces.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn runner_errors_and_dropped_model_or_tool_futures_close_on_owner_drop() {
    for mode in ["error", "model", "tool"] {
        let (sink, owner, observer) = setup();
        let step = match mode {
            "error" => Step::Error,
            "model" => Step::Pending,
            _ => response(vec![RunItem::ToolCall { call: call() }]),
        };
        let mut agent = agent("agent", vec![step]);
        agent.tools.push(Arc::new(MockTool {
            definition: definition("tool"),
            pending: true,
            fail: false,
        }));
        let runner = Runner::new(agent, config(observer)).unwrap();
        let mut future = Box::pin(runner.run(context(), request(), Arc::new(HostSink)));
        if mode == "error" {
            assert!(future.as_mut().await.is_err());
        } else {
            std::future::poll_fn(|cx| {
                assert!(std::future::Future::poll(future.as_mut(), cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
        }
        drop(future);
        assert!(sink.traces.lock().unwrap().is_empty());
        drop(owner);
        let trace = assert_closed(&sink);
        let data = trace
            .spans
            .iter()
            .find_map(|span| match &span.data {
                Some(SpanData::Generation(data)) => Some(data),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            data.status,
            match mode {
                "error" => "failed",
                "model" => "interrupted",
                _ => "completed",
            }
        );
        if mode == "tool" {
            assert!(trace.spans.iter().any(|span| matches!(&span.data,
                Some(SpanData::Function { is_error: true, output, .. })
                    if output.contains("interrupted")
            )));
        }
    }
}

#[tokio::test]
async fn runner_handoff_switches_generation_parent_on_shared_root() {
    let (sink, owner, observer) = setup();
    let target = Arc::new(agent("target", vec![done()]));
    let mut call = call();
    call.name = "transfer".into();
    let mut source = agent("source", vec![response(vec![RunItem::ToolCall { call }])]);
    source.handoffs.push(Handoff {
        definition: definition("transfer"),
        target,
    });
    let runner = Runner::new(source, config(observer)).unwrap();
    runner
        .run(context(), request(), Arc::new(HostSink))
        .await
        .unwrap();
    owner.finish();
    let trace = assert_closed(&sink);
    let agents: Vec<_> = trace
        .spans
        .iter()
        .filter(|span| span.name == "agent")
        .collect();
    assert_eq!(agents.len(), 2);
    assert!(agents.iter().all(|span| span.parent_id == trace.id));
    let generations: Vec<_> = trace
        .spans
        .iter()
        .filter(|span| span.name == "generation")
        .collect();
    assert_eq!(generations.len(), 2);
    assert_eq!(generations[0].parent_id, agents[0].id);
    assert_eq!(generations[1].parent_id, agents[1].id);
    let handoff = trace
        .spans
        .iter()
        .find(|span| span.name == "handoff")
        .unwrap();
    assert_eq!(handoff.parent_id, agents[0].id);
    assert!(
        matches!(&handoff.data, Some(SpanData::Handoff { from_agent, to_agent }) if from_agent == "source" && to_agent == "target")
    );
}

fn generation() -> GenerationRecord {
    GenerationRecord {
        id: "generation-id".into(),
        agent: "source".into(),
        provider: "fixture".into(),
        resolved_model: "model".into(),
        input_tokens_include_cache: None,
        task_id: None,
        cost_usd: None,
        turn: 1,
        request: ModelRequest {
            model: "model".into(),
            instructions: String::new(),
            input: vec![],
            tools: vec![],
            settings: Default::default(),
            output_schema: None,
            output_schema_name: String::new(),
            output_schema_strict: false,
        },
        response: None,
        error: None,
        retry_reason: None,
        status: GenerationStatus::Started,
        retry_after: None,
        fallback_model: None,
        started_at: SystemTime::now(),
        ended_at: None,
        latency: Duration::ZERO,
    }
}
#[tokio::test]
async fn late_generation_keeps_parent_and_repeated_starts_are_safe() {
    let (sink, owner, observer) = setup();
    let context = context();
    let started = Observation::AgentStarted {
        agent: "source".into(),
    };
    observer.observe(&context, started.clone()).await.unwrap();
    observer.observe(&context, started).await.unwrap();
    let mut record = generation();
    observer.start(&context, &record);
    observer.start(&context, &record);
    observer
        .observe(
            &context,
            Observation::ToolStarted {
                agent: "source".into(),
                call: call(),
            },
        )
        .await
        .unwrap();
    observer
        .observe(
            &context,
            Observation::ToolStarted {
                agent: "source".into(),
                call: call(),
            },
        )
        .await
        .unwrap();
    observer
        .observe(
            &context,
            Observation::Handoff {
                from: "source".into(),
                to: "target".into(),
            },
        )
        .await
        .unwrap();
    observer
        .observe(
            &context,
            Observation::AgentStarted {
                agent: "target".into(),
            },
        )
        .await
        .unwrap();
    record.status = GenerationStatus::Interrupted;
    record.ended_at = Some(SystemTime::now());
    observer.end(&context, &record);
    observer.end(&context, &record);
    observer
        .observe(
            &context,
            Observation::RawToolOutput {
                call: call(),
                output: ToolOutput {
                    content: vec![],
                    is_error: false,
                    should_pause: false,
                },
            },
        )
        .await
        .unwrap();
    owner.finish();
    observer
        .observe(
            &context,
            Observation::AgentStarted {
                agent: "ignored".into(),
            },
        )
        .await
        .unwrap();
    observer.start(&context, &record);
    let trace = assert_closed(&sink);
    assert_eq!(trace.spans.len(), 5);
    let source = &trace.spans[0];
    let generation = trace
        .spans
        .iter()
        .find(|span| span.name == "generation")
        .unwrap();
    let function = trace
        .spans
        .iter()
        .find(|span| span.name == "function")
        .unwrap();
    assert_eq!(generation.parent_id, source.id);
    assert_eq!(function.parent_id, source.id);
    let events = sink.events.lock().unwrap();
    let source_end = events
        .iter()
        .position(|(kind, id)| kind == "end" && id == &source.id)
        .unwrap();
    let child_end = events
        .iter()
        .position(|(kind, id)| kind == "end" && id == &generation.id)
        .unwrap();
    assert!(source_end < child_end);
}

#[tokio::test]
async fn compaction_records_only_observed_success_counts_and_never_error_content() {
    let (sink, owner, observer) = setup();
    let context = context();
    for end in [Some(true), Some(false), None] {
        observer
            .observe(
                &context,
                Observation::CompactionStarted {
                    context_tokens: 100,
                    target_tokens: 25,
                },
            )
            .await
            .unwrap();
        observer
            .observe(
                &context,
                Observation::CompactionStarted {
                    context_tokens: 200,
                    target_tokens: 20,
                },
            )
            .await
            .unwrap();
        match end {
            Some(true) => observer
                .observe(
                    &context,
                    Observation::Compacted {
                        before_items: 8,
                        after_items: 2,
                        context_tokens: 30,
                    },
                )
                .await
                .unwrap(),
            Some(false) => observer
                .observe(
                    &context,
                    Observation::CompactionFailed {
                        error: Error::new(ErrorCategory::Provider, "private-error").info,
                    },
                )
                .await
                .unwrap(),
            None => {}
        }
    }
    owner.finish();
    let trace = assert_closed(&sink);
    assert_eq!(trace.spans.len(), 6);
    // Every new start replaces an unterminated attempt. Only the second
    // attempt observed success, so the first must not inherit its counts.
    assert!(matches!(
        trace.spans[1].data,
        Some(SpanData::Compaction {
            tokens_before: 200,
            tokens_after: 30
        })
    ));
    for index in [0, 2, 3, 4, 5] {
        assert!(trace.spans[index].data.is_none());
    }
    assert!(!format!("{trace:?}").contains("private-error"));
}

#[tokio::test]
async fn processors_can_reenter_without_locks_and_owner_finish_is_queued() {
    #[derive(Default)]
    struct Reentrant {
        observer: Mutex<Option<Arc<RuntimeTracing>>>,
        owner: Mutex<Option<RunTrace>>,
        ended: Mutex<usize>,
    }
    impl TraceProcessor for Reentrant {
        fn span_start(&self, _: &Span) {
            let observer = self.observer.lock().unwrap().clone().unwrap();
            observer.start(&context(), &generation());
            let owner = self.owner.lock().unwrap().take();
            drop(owner);
        }
        fn trace_end(&self, _: &Trace) {
            *self.ended.lock().unwrap() += 1;
        }
    }
    let sink = Arc::new(Reentrant::default());
    let owner = RunTrace::new(TraceSession::new("root", sink.clone()));
    let observer = owner.observer();
    *sink.observer.lock().unwrap() = Some(observer.clone());
    *sink.owner.lock().unwrap() = Some(owner);
    observer
        .observe(
            &context(),
            Observation::AgentStarted {
                agent: "agent".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(*sink.ended.lock().unwrap(), 1);
    sink.observer.lock().unwrap().take();
}

#[tokio::test]
async fn runner_compaction_counts_are_child_of_active_agent() {
    struct Compact;
    impl Compactor for Compact {
        fn compact<'a>(
            &'a self,
            _: &'a Context,
            request: CompactionRequest,
        ) -> BoxFuture<'a, Result<CompactedHistory, Error>> {
            Box::pin(async move {
                assert_eq!(request.context_tokens, 100);
                Ok(CompactedHistory {
                    history: request.history,
                    context_tokens: 30,
                    usage: Usage::default(),
                    cost: 0.0,
                })
            })
        }
    }
    let (sink, owner, observer) = setup();
    let mut first = response(vec![RunItem::ToolCall { call: call() }]);
    if let Step::Response(response) = &mut first {
        response.usage.context_tokens = Some(100);
    }
    let mut agent = agent("agent", vec![first, done()]);
    agent.tools.push(Arc::new(MockTool {
        definition: definition("tool"),
        pending: false,
        fail: false,
    }));
    let mut config = config(observer);
    // Empty initial history causes a local no-op start without a terminal hook.
    // A later successful attempt must not inherit that stale root span/count.
    config.local_compaction.trigger_tokens = 100;
    config.local_compaction.target_tokens = 50;
    config.compaction = Some(CompactionConfig {
        trigger_tokens: 50,
        target_tokens: 25,
        compactor: Arc::new(Compact),
    });
    let runner = Runner::new(agent, config).unwrap();
    runner
        .run(context(), request(), Arc::new(HostSink))
        .await
        .unwrap();
    owner.finish();
    let trace = assert_closed(&sink);
    let agent = trace
        .spans
        .iter()
        .find(|span| span.name == "agent")
        .unwrap();
    let compact = trace
        .spans
        .iter()
        .find(|span| {
            matches!(
                span.data,
                Some(SpanData::Compaction {
                    tokens_before: 100,
                    tokens_after: 30
                })
            )
        })
        .unwrap();
    assert_eq!(compact.parent_id, agent.id);
    assert!(matches!(
        compact.data,
        Some(SpanData::Compaction {
            tokens_before: 100,
            tokens_after: 30
        })
    ));
}

#[tokio::test]
async fn external_child_retains_shared_root_after_owner_finish() {
    let sink = Arc::new(Recorder::default());
    let session = TraceSession::new("root", sink.clone());
    let parent = session.span("manual", None);
    let child = parent.child("late", None);
    let parent_id = parent.id().to_owned();
    let owner = RunTrace::new(session);
    let observer = owner.observer();
    observer
        .observe(
            &context(),
            Observation::AgentStarted {
                agent: "agent".into(),
            },
        )
        .await
        .unwrap();
    parent.finish();
    owner.finish();
    assert!(sink.traces.lock().unwrap().is_empty());
    assert_eq!(child.finish().parent_id, parent_id);
    let trace = assert_closed(&sink);
    assert_eq!(trace.spans.len(), 3);
    assert_eq!(trace.spans[2].parent_id, trace.id);
}

#[tokio::test]
async fn owned_run_wrapper_orders_cleanup_on_completion_and_cancellation() {
    for pending in [false, true] {
        let (sink, owner, observer) = setup();
        let runner = Runner::new(
            agent("agent", vec![if pending { Step::Pending } else { done() }]),
            config(observer),
        )
        .unwrap();
        let future = runner.run(context(), request(), Arc::new(HostSink));
        let mut traced = Box::pin(owner.run(future));
        if pending {
            std::future::poll_fn(|cx| {
                assert!(std::future::Future::poll(traced.as_mut(), cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
        } else {
            traced.as_mut().await.unwrap();
        }
        drop(traced);
        let trace = assert_closed(&sink);
        let generation = trace
            .spans
            .iter()
            .find_map(|span| match &span.data {
                Some(SpanData::Generation(data)) => Some(data),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            generation.status,
            if pending { "interrupted" } else { "completed" }
        );
        // Observer Arcs retained in the runner cannot keep the completed trace open.
        drop(runner);
        assert_eq!(sink.traces.lock().unwrap().len(), 1);
    }
}
