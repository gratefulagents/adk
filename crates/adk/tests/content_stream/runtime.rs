use super::{Buffer, ContentEvent, ContentStream};
use adk::{
    content_stream::ContentGenerationObserver,
    core::*,
    runtime::{tracing::*, *},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

fn response() -> ModelResponse {
    ModelResponse {
        items: vec![
            RunItem::Reasoning {
                reasoning: Reasoning {
                    text: "full secret reasoning".into(),
                    ..Default::default()
                },
            },
            RunItem::Reasoning {
                reasoning: Reasoning {
                    text: "second item".into(),
                    ..Default::default()
                },
            },
            RunItem::Reasoning {
                reasoning: Reasoning {
                    signature: "opaque signature".into(),
                    encrypted_content: "opaque encrypted".into(),
                    redacted_data: "opaque redacted".into(),
                    ..Default::default()
                },
            },
            RunItem::Message {
                message: Message {
                    role: Role::Assistant,
                    content: vec![Content::Text {
                        text: "done".into(),
                    }],
                },
            },
        ],
        usage: Usage {
            input_tokens: 5,
            output_tokens: 2,
            cache_read_tokens: 3,
            cache_creation_tokens: 1,
            ..Default::default()
        },
        end_turn: None,
        response_id: None,
        metadata: Default::default(),
        raw: None,
        snapshot_raw: None,
        snapshot_projection: None,
    }
}
fn request() -> RunRequest {
    RunRequest {
        input: vec![],
        input_provenance: vec![],
        policy: RunPolicy::default(),
    }
}
fn context(token: &CancellationToken) -> Context {
    Context {
        run_id: "content".into(),
        cancellation: Arc::new(token.clone()),
        deadline: None,
    }
}
fn delta(text: &str) -> Step {
    Step::Event(ModelEvent::ReasoningDelta { delta: text.into() })
}
fn complete() -> Step {
    Step::Event(ModelEvent::Complete {
        response: response(),
    })
}
enum Step {
    Event(ModelEvent),
    Fail,
    Wait,
}
struct Scripted {
    streams: Mutex<VecDeque<Vec<Step>>>,
    starts: AtomicUsize,
    completes: AtomicUsize,
    entered: Arc<tokio::sync::Notify>,
}
impl Scripted {
    fn new(streams: Vec<Vec<Step>>) -> Arc<Self> {
        Arc::new(Self {
            streams: Mutex::new(streams.into()),
            starts: AtomicUsize::new(0),
            completes: AtomicUsize::new(0),
            entered: Arc::new(tokio::sync::Notify::new()),
        })
    }
}
impl Model for Scripted {
    fn provider(&self) -> &str {
        "mock"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            self.completes.fetch_add(1, Ordering::SeqCst);
            Ok(response())
        })
    }
}
struct Events {
    steps: VecDeque<Step>,
    entered: Arc<tokio::sync::Notify>,
}
impl ModelStream for Events {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async move {
            match self.steps.pop_front() {
                Some(Step::Event(event)) => Ok(Some(event)),
                Some(Step::Fail) => Err(Error::new(ErrorCategory::Internal, "failure")),
                Some(Step::Wait) => {
                    self.entered.notify_one();
                    std::future::pending().await
                }
                None => Ok(None),
            }
        })
    }
}
impl StreamingModel for Scripted {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            self.starts.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Events {
                steps: self
                    .streams
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected retry")
                    .into(),
                entered: self.entered.clone(),
            }) as Box<dyn ModelStream>)
        })
    }
}
#[derive(Default)]
struct Sink(Mutex<Vec<RunEvent>>);
impl Host for Sink {
    fn emit<'a>(&'a self, _: &'a Context, event: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(event);
            Ok(())
        })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { panic!("unexpected approval") })
    }
}
struct ZeroCost;
impl CostEstimator for ZeroCost {
    fn cost(&self, _: &str, _: &Usage) -> f64 {
        0.0
    }
}
fn setup() -> (Arc<ContentGenerationObserver>, Buffer, Arc<ContentStream>) {
    let buffer = Buffer::default();
    let stream = Arc::new(ContentStream::new(buffer.clone()));
    (
        Arc::new(ContentGenerationObserver::new(stream.clone())),
        buffer,
        stream,
    )
}
fn config(observer: Arc<dyn GenerationObserver>) -> RunnerConfig {
    RunnerConfig {
        blocking_reasoning: true,
        generation_observer: Some(observer),
        cost_estimator: Some(Arc::new(ZeroCost)),
        retry: RetryPolicy {
            max_retries: 0,
            ..Default::default()
        },
        ..Default::default()
    }
}
fn runner(model: &Arc<Scripted>, config: RunnerConfig) -> Runner {
    Runner::new(
        AgentConfig::new(
            "worker",
            ModelBinding::streaming("mock/model", model.clone()),
        ),
        config,
    )
    .unwrap()
}
fn events(buffer: &Buffer) -> Vec<ContentEvent> {
    String::from_utf8(buffer.0.lock().unwrap().clone())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn assert_correlated(events: &[ContentEvent]) {
    assert!(!events[0].tool_use_id.is_empty());
    for event in events {
        assert_eq!(event.tool_use_id, events[0].tool_use_id);
        assert_eq!(event.agent_name, "worker");
    }
}

#[tokio::test]
async fn blocking_run_correlates_ordered_chunks_and_full_accepted_items() {
    let (observer, buffer, stream) = setup();
    let subscription = stream.subscribe(32);
    let model = Scripted::new(vec![vec![
        delta(""),
        delta("live secret"),
        Step::Event(ModelEvent::TextDelta {
            delta: "hidden".into(),
        }),
        delta(" residue"),
        complete(),
    ]]);
    runner(&model, config(observer))
        .run(
            context(&CancellationToken::new()),
            request(),
            Arc::new(Sink::default()),
        )
        .await
        .unwrap();
    let events = events(&buffer);
    assert_correlated(&events);
    assert_eq!(
        events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
        [
            "llm_attempt",
            "assistant_thinking_delta",
            "assistant_thinking_delta",
            "llm_attempt",
            "assistant_thinking",
            "assistant_thinking"
        ]
    );
    assert_eq!(
        (&events[1].message, &events[2].message),
        (&"live secret".to_owned(), &" residue".to_owned())
    );
    assert_eq!(events[4].message, "full secret reasoning");
    assert_eq!(events[5].message, "second item");
    assert_eq!(events[0].status, "started");
    assert!(!events[0].usage_available);
    assert!(!events[0].cost_known);
    assert_eq!(events[3].status, "completed");
    assert_eq!(
        (
            events[3].input_tokens,
            events[3].output_tokens,
            events[3].total_tokens
        ),
        (5, 2, 7)
    );
    assert_eq!(events[3].prompt_tokens, 0);
    assert!(
        events[3].usage_available
            && events[3].has_prompt_tokens
            && events[3].has_completion_tokens
            && events[3].has_total_tokens
    );
    assert_eq!(subscription.receiver.try_iter().count(), events.len());
}

#[tokio::test]
async fn attempt_events_match_pinned_sdk_real_runner_objects() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/content-stream/observations.json"
    ))
    .unwrap();
    for case in fixture["thinkingAttempts"].as_array().unwrap() {
        let failed = case["failed"].as_bool().unwrap();
        let (observer, buffer, _) = setup();
        let model = Scripted::new(vec![if failed {
            vec![Step::Fail]
        } else {
            vec![complete()]
        }]);
        let result = runner(&model, config(observer))
            .run(
                context(&CancellationToken::new()),
                request(),
                Arc::new(Sink::default()),
            )
            .await;
        assert_eq!(result.is_err(), failed);
        let events = events(&buffer);
        assert_correlated(&events);
        let normalized: Vec<_> = events
            .into_iter()
            .filter(|e| e.kind == "llm_attempt")
            .map(|mut event| {
                event.timestamp = Default::default();
                event.tool_use_id = "attempt".into();
                event.attempt_latency_ms = 0;
                serde_json::to_value(event).unwrap()
            })
            .collect();
        assert_eq!(json!(normalized), case["events"]);
    }
}

#[tokio::test]
async fn streamed_run_has_native_reasoning_but_zero_jsonl_deltas() {
    let (observer, buffer, _) = setup();
    let model = Scripted::new(vec![vec![delta("native live"), complete()]]);
    let sink = Arc::new(Sink::default());
    runner(&model, config(observer))
        .stream(context(&CancellationToken::new()), request(), sink.clone())
        .finish()
        .await
        .unwrap();
    assert_eq!(
        sink.0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(
                e,
                RunEvent::Model {
                    event: ModelEvent::ReasoningDelta { .. }
                }
            ))
            .count(),
        1
    );
    let events = events(&buffer);
    assert_correlated(&events);
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == "assistant_thinking_delta")
            .count(),
        0
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == "assistant_thinking")
            .count(),
        2
    );
}

#[tokio::test]
async fn complete_only_and_default_blocking_do_not_invent_chunks() {
    for streaming_binding in [false, true] {
        let (observer, buffer, _) = setup();
        let model = Scripted::new(vec![]);
        let mut config = config(observer);
        config.blocking_reasoning = !streaming_binding;
        let binding = if streaming_binding {
            ModelBinding::streaming("mock/model", model.clone())
        } else {
            ModelBinding::complete("mock/model", model.clone())
        };
        Runner::new(AgentConfig::new("worker", binding), config)
            .unwrap()
            .run(
                context(&CancellationToken::new()),
                request(),
                Arc::new(Sink::default()),
            )
            .await
            .unwrap();
        let events = events(&buffer);
        assert_correlated(&events);
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == "assistant_thinking_delta")
                .count(),
            0
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == "assistant_thinking")
                .count(),
            2
        );
        assert_eq!(model.starts.load(Ordering::SeqCst), 0);
        assert_eq!(model.completes.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn retained_failed_responses_never_emit_final_thinking_or_usage() {
    let (observer, buffer, _) = setup();
    let model = Scripted::new(vec![vec![
        delta("first"),
        delta("tail"),
        complete(),
        Step::Fail,
    ]]);
    let error = runner(&model, config(observer))
        .run(
            context(&CancellationToken::new()),
            request(),
            Arc::new(Sink::default()),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.partial.unwrap().responses.len(), 1);
    let events = events(&buffer);
    assert_correlated(&events);
    assert_eq!(events.len(), 4);
    assert_eq!(events[2].message, "tail");
    let end = &events[3];
    assert_eq!(end.status, "failed");
    assert!(!end.usage_available);
    assert!(!end.cost_known);
    assert_eq!(end.input_tokens, 0);
    assert!(end.is_error);
    assert_eq!(end.output, "failure");
    assert_eq!(end.reason, "error");
}

#[tokio::test]
async fn cancellation_and_drop_flush_residue_without_retained_final_or_duplicates() {
    for cancel in [true, false] {
        let (observer, buffer, _) = setup();
        let model = Scripted::new(vec![vec![
            delta("first"),
            delta("tail"),
            complete(),
            Step::Wait,
        ]]);
        let runner = runner(&model, config(observer));
        let token = CancellationToken::new();
        {
            let future = runner.run(context(&token), request(), Arc::new(Sink::default()));
            tokio::pin!(future);
            tokio::select! { result = &mut future => panic!("unexpected completion: {}", result.is_ok()), _ = model.entered.notified() => {} }
            if cancel {
                token.cancel();
                let error = future.await.err().unwrap();
                assert_eq!(error.error.info.category, ErrorCategory::Cancelled);
                assert_eq!(error.partial.unwrap().responses.len(), 1);
            }
        }
        let events = events(&buffer);
        assert_correlated(&events);
        assert_eq!(events.len(), 4);
        assert_eq!(events[2].message, "tail");
        assert_eq!(
            events[3].status,
            if cancel { "failed" } else { "interrupted" }
        );
        assert!(!events[3].usage_available);
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn composition_preserves_trace_and_content_remains_trusted() {
    use adk::{
        observability::CaptureMode,
        tracestore::{FilesystemTraceStore, RunMetadata},
        tracewriter::{Options, TraceWriter},
    };
    let root = tempfile::tempdir().unwrap();
    let writer = Arc::new(TraceWriter::new(
        Arc::new(FilesystemTraceStore::new(root.path()).unwrap()),
        "content",
        Options {
            capture: CaptureMode::Full,
            redactors: vec![Arc::new(|text| text.replace("secret", "REDACTED"))],
        },
    ));
    let path = writer
        .init_run(&RunMetadata {
            run_id: "content".into(),
            ..Default::default()
        })
        .unwrap();
    let (observer, buffer, _) = setup();
    let observer = Arc::new(CompositeGenerationObserver::new(vec![
        writer.clone(),
        observer,
    ]));
    let model = Scripted::new(vec![vec![delta("live secret"), complete()]]);
    runner(&model, config(observer))
        .run(
            context(&CancellationToken::new()),
            request(),
            Arc::new(Sink::default()),
        )
        .await
        .unwrap();
    let events = events(&buffer);
    assert_correlated(&events);
    assert_eq!(events[1].message, "live secret");
    assert_eq!(events[3].message, "full secret reasoning");
    let trace = std::fs::read_to_string(path.join("llm_calls.jsonl")).unwrap();
    let calls: Vec<Value> = trace
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["type"], "generation_start");
    assert_eq!(calls[1]["type"], "generation_end");
    assert_eq!(calls[1]["status"], "completed");
    assert_eq!(calls[0]["span_id"], events[0].tool_use_id);
    assert_eq!(calls[1]["span_id"], events[0].tool_use_id);
    assert!(trace.contains("full REDACTED reasoning"));
    assert!(!trace.contains("full secret reasoning"));
    assert_eq!(writer.health().write_errors, 0);
}

#[tokio::test]
async fn retried_attempts_have_distinct_ids_and_only_success_has_final_thinking() {
    let (observer, buffer, _) = setup();
    let model = Scripted::new(vec![vec![Step::Fail], vec![delta("live"), complete()]]);
    let mut config = config(observer);
    config.retry.max_retries = 1;
    config.retry.retryable = |_| true;
    config.retry.initial_delay = std::time::Duration::ZERO;
    runner(&model, config)
        .run(
            context(&CancellationToken::new()),
            request(),
            Arc::new(Sink::default()),
        )
        .await
        .unwrap();
    let events = events(&buffer);
    assert_eq!(events.len(), 7);
    assert_correlated(&events[..2]);
    assert_correlated(&events[2..]);
    assert_ne!(events[0].tool_use_id, events[2].tool_use_id);
    assert_eq!(events[1].status, "retrying");
    assert!(events[1].retry_planned);
    assert!(!events[1].usage_available);
    assert_eq!(events[4].status, "completed");
    assert_eq!(events[5].kind, "assistant_thinking");
}
