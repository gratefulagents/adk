#[path = "generation_content/completion.rs"]
mod completion;

use adk_core::*;
use adk_runtime::{tracing::*, *};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn answer() -> ModelResponse {
    ModelResponse {
        items: vec![RunItem::Message {
            message: Message {
                role: Role::Assistant,
                content: vec![Content::Text {
                    text: "done".into(),
                }],
            },
        }],
        usage: Usage {
            input_tokens: 7,
            output_tokens: 2,
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
fn reasoning(delta: &str) -> Step {
    Step::Event(ModelEvent::ReasoningDelta {
        delta: delta.into(),
    })
}
fn complete() -> Step {
    Step::Event(ModelEvent::Complete { response: answer() })
}
enum Step {
    Event(ModelEvent),
    Fail,
    Wait,
    Wake,
}
#[derive(Default)]
struct Offline {
    streams: Mutex<VecDeque<Vec<Step>>>,
    requests: Mutex<Vec<ModelRequest>>,
    completes: AtomicUsize,
    starts: AtomicUsize,
    drops: Arc<AtomicUsize>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    wake: Arc<tokio::sync::Notify>,
}
impl Offline {
    fn new(streams: Vec<Vec<Step>>) -> Arc<Self> {
        Arc::new(Self {
            streams: Mutex::new(streams.into()),
            ..Default::default()
        })
    }
}
impl Model for Offline {
    fn provider(&self) -> &str {
        "offline"
    }
    fn retry_advice(&self, _: &Error) -> Option<ModelRetryAdvice> {
        Some(ModelRetryAdvice {
            should_retry: true,
            retry_after: Duration::ZERO,
            reason: "test".into(),
        })
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            self.completes.fetch_add(1, Ordering::SeqCst);
            self.requests.lock().unwrap().push(request);
            Ok(answer())
        })
    }
}
struct Events {
    steps: VecDeque<Step>,
    drops: Arc<AtomicUsize>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    wake: Arc<tokio::sync::Notify>,
}
impl Drop for Events {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
impl ModelStream for Events {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async move {
            loop {
                match self.steps.pop_front() {
                    Some(Step::Event(event)) => return Ok(Some(event)),
                    Some(Step::Fail) => {
                        return Err(Error::new(ErrorCategory::Provider, "offline failure"));
                    }
                    Some(Step::Wait) => {
                        self.entered.notify_one();
                        self.release.notified().await;
                    }
                    Some(Step::Wake) => {
                        self.wake.notify_one();
                        tokio::task::yield_now().await;
                    }
                    None => return Ok(None),
                }
            }
        })
    }
}
impl StreamingModel for Offline {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            self.starts.fetch_add(1, Ordering::SeqCst);
            self.requests.lock().unwrap().push(request);
            Ok(Box::new(Events {
                steps: self
                    .streams
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected retry")
                    .into(),
                drops: self.drops.clone(),
                entered: self.entered.clone(),
                release: self.release.clone(),
                wake: self.wake.clone(),
            }) as Box<dyn ModelStream>)
        })
    }
}
#[derive(Clone)]
struct Seen {
    observer: u8,
    event: String,
    record: GenerationRecord,
}
struct Observer {
    id: u8,
    seen: Arc<Mutex<Vec<Seen>>>,
}
impl Observer {
    fn push(&self, ctx: &Context, record: &GenerationRecord, event: &str) {
        assert_eq!(ctx.run_id, "content");
        self.seen.lock().unwrap().push(Seen {
            observer: self.id,
            event: event.into(),
            record: record.clone(),
        });
    }
}
impl GenerationObserver for Observer {
    fn start(&self, c: &Context, r: &GenerationRecord) {
        self.push(c, r, "start");
    }
    fn end(&self, c: &Context, r: &GenerationRecord) {
        self.push(c, r, "end");
    }
    fn returned(&self, c: &Context, r: &GenerationRecord) {
        self.push(c, r, "returned");
    }
    fn accepted(&self, c: &Context, r: &GenerationRecord) {
        self.push(c, r, "accepted");
    }
    fn reasoning_delta(&self, c: &Context, r: &GenerationRecord, delta: &str) {
        self.push(c, r, &format!("delta:{delta}"));
    }
}
#[derive(Default)]
struct Sink {
    events: Mutex<Vec<RunEvent>>,
    accepted: AtomicUsize,
}
impl Host for Sink {
    fn emit<'a>(&'a self, _: &'a Context, event: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.events.lock().unwrap().push(event);
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
impl RunHooks for Sink {
    fn durable_observer(&self) -> bool {
        true
    }
    fn observe<'a>(
        &'a self,
        _: &'a Context,
        event: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            if matches!(event, Observation::ModelAccepted { .. }) {
                self.accepted.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        })
    }
}
fn config(seen: &Arc<Mutex<Vec<Seen>>>, sink: &Arc<Sink>) -> RunnerConfig {
    RunnerConfig {
        blocking_reasoning: true,
        generation_observer: Some(Arc::new(CompositeGenerationObserver::new(vec![
            Arc::new(Observer {
                id: 1,
                seen: seen.clone(),
            }),
            Arc::new(Observer {
                id: 2,
                seen: seen.clone(),
            }),
        ]))),
        hooks: Some(sink.clone()),
        retry: RetryPolicy {
            max_retries: 1,
            initial_delay: Duration::ZERO,
            ..Default::default()
        },
        ..Default::default()
    }
}
fn runner(model: &Arc<Offline>, config: RunnerConfig) -> Runner {
    Runner::new(
        AgentConfig::new("worker", ModelBinding::streaming("offline", model.clone())),
        config,
    )
    .unwrap()
}
fn check_order(seen: &[Seen], expected: &[&str]) {
    assert_eq!(seen.len(), expected.len() * 2);
    for (pair, event) in seen.chunks_exact(2).zip(expected) {
        assert_eq!((pair[0].observer, pair[1].observer), (1, 2));
        assert_eq!(pair[0].event, *event);
        assert_eq!(pair[1].event, *event);
        assert_eq!(pair[0].record.id, pair[1].record.id);
    }
}
#[tokio::test]
async fn blocking_reasoning_is_ordered_correlated_and_hides_intermediate_events() {
    let model = Offline::new(vec![vec![
        reasoning(""),
        reasoning("let me"),
        Step::Event(ModelEvent::TextDelta {
            delta: "hidden".into(),
        }),
        Step::Event(ModelEvent::ToolArgumentsDelta {
            call_id: "c".into(),
            delta: "{}".into(),
        }),
        reasoning(" think"),
        complete(),
    ]]);
    let seen = Arc::new(Mutex::new(vec![]));
    let sink = Arc::new(Sink::default());
    let result = runner(&model, config(&seen, &sink))
        .run(context(&CancellationToken::new()), request(), sink.clone())
        .await
        .unwrap();
    assert_eq!(result.result.responses.len(), 1);
    let seen = seen.lock().unwrap();
    check_order(
        &seen,
        &[
            "start",
            "delta:let me",
            "delta: think",
            "returned",
            "end",
            "accepted",
        ],
    );
    for event in seen.iter() {
        assert_eq!(event.record.id, seen[0].record.id);
        assert_eq!(event.record.agent, "worker");
    }
    assert!(seen[6].record.ended_at.is_some());
    assert_eq!(
        seen.last().unwrap().record.status,
        GenerationStatus::Completed
    );
    assert_eq!(seen.last().unwrap().record.response, Some(answer()));
    assert_eq!(sink.accepted.load(Ordering::SeqCst), 1);
    let events = sink.events.lock().unwrap();
    let model_events: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let RunEvent::Model { event } = e {
                Some(event)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        model_events,
        vec![&ModelEvent::Complete { response: answer() }]
    );
    assert_eq!(model.completes.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn opt_in_default_and_complete_only_select_the_expected_method() {
    for (opt_in, streaming_binding) in [(false, true), (false, false), (true, false), (true, true)]
    {
        let model = Offline::new(vec![vec![complete()]]);
        let seen = Arc::new(Mutex::new(vec![]));
        let sink = Arc::new(Sink::default());
        let mut cfg = config(&seen, &sink);
        cfg.blocking_reasoning = opt_in;
        let binding = if streaming_binding {
            ModelBinding::streaming("offline", model.clone())
        } else {
            ModelBinding::complete("offline", model.clone())
        };
        Runner::new(AgentConfig::new("worker", binding), cfg)
            .unwrap()
            .run(context(&CancellationToken::new()), request(), sink)
            .await
            .unwrap();
        assert_eq!(
            model.starts.load(Ordering::SeqCst),
            usize::from(opt_in && streaming_binding)
        );
        assert_eq!(
            model.completes.load(Ordering::SeqCst),
            usize::from(!opt_in || !streaming_binding)
        );
        check_order(
            &seen.lock().unwrap(),
            &["start", "returned", "end", "accepted"],
        );
    }
    assert!(!RunnerConfig::default().blocking_reasoning);
}
#[tokio::test]
async fn streamed_run_forwards_reasoning_without_observer_duplicates() {
    let model = Offline::new(vec![vec![reasoning("live"), complete()]]);
    let seen = Arc::new(Mutex::new(vec![]));
    let sink = Arc::new(Sink::default());
    runner(&model, config(&seen, &sink))
        .stream(context(&CancellationToken::new()), request(), sink.clone())
        .finish()
        .await
        .unwrap();
    check_order(
        &seen.lock().unwrap(),
        &["start", "returned", "end", "accepted"],
    );
    assert_eq!(
        sink.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(
                event,
                RunEvent::Model {
                    event: ModelEvent::ReasoningDelta { .. }
                }
            ))
            .count(),
        1
    );
}
#[tokio::test]
async fn hidden_text_and_tools_are_not_committed_or_retained_on_retry() {
    let model = Offline::new(vec![
        vec![
            reasoning(""),
            Step::Event(ModelEvent::TextDelta {
                delta: "phantom".into(),
            }),
            Step::Event(ModelEvent::ToolArgumentsDelta {
                call_id: "c".into(),
                delta: "{}".into(),
            }),
            Step::Event(ModelEvent::ItemDone {
                item: answer().items[0].clone(),
            }),
            Step::Fail,
        ],
        vec![complete()],
    ]);
    let seen = Arc::new(Mutex::new(vec![]));
    let sink = Arc::new(Sink::default());
    let result = runner(&model, config(&seen, &sink))
        .run(context(&CancellationToken::new()), request(), sink.clone())
        .await
        .unwrap();
    assert_eq!(result.result.new_items, answer().items);
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests[0].input, requests[1].input);
    let seen = seen.lock().unwrap();
    check_order(
        &seen,
        &[
            "start", "returned", "end", "start", "returned", "end", "accepted",
        ],
    );
    assert_eq!(seen[4].record.status, GenerationStatus::Retrying);
    assert_ne!(seen[0].record.id, seen[6].record.id);
    assert_eq!(seen[6].record.id, seen[12].record.id);
    assert_eq!(sink.accepted.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn surfaced_reasoning_blocks_retry_and_failed_retained_completion_is_not_accepted() {
    for (retained, surfaced) in [(false, true), (true, true), (true, false)] {
        let mut steps = if surfaced {
            vec![reasoning("visible")]
        } else {
            vec![]
        };
        if retained {
            steps.push(complete());
        }
        steps.push(Step::Fail);
        let model = Offline::new(vec![steps]);
        let seen = Arc::new(Mutex::new(vec![]));
        let sink = Arc::new(Sink::default());
        let error = runner(&model, config(&seen, &sink))
            .run(context(&CancellationToken::new()), request(), sink.clone())
            .await
            .err()
            .unwrap();
        let partial = error.partial.unwrap();
        assert_eq!(partial.responses.len(), usize::from(retained));
        assert_eq!(partial.usage.input_tokens, if retained { 7 } else { 0 });
        assert_eq!(model.starts.load(Ordering::SeqCst), 1);
        assert_eq!(sink.accepted.load(Ordering::SeqCst), 0);
        let seen = seen.lock().unwrap();
        check_order(
            &seen,
            if surfaced {
                &["start", "delta:visible", "returned", "end"]
            } else {
                &["start", "returned", "end"]
            },
        );
        assert_eq!(seen.last().unwrap().record.status, GenerationStatus::Failed);
        assert_eq!(seen.last().unwrap().record.response.is_some(), retained);
    }
}
#[tokio::test]
async fn cancellation_and_dropped_future_flush_and_close_once() {
    for cancel in [false, true] {
        let model = Offline::new(vec![vec![reasoning("partial"), Step::Wait, complete()]]);
        let seen = Arc::new(Mutex::new(vec![]));
        let sink = Arc::new(Sink::default());
        let runner = runner(&model, config(&seen, &sink));
        let token = CancellationToken::new();
        {
            let future = runner.run(context(&token), request(), sink.clone());
            tokio::pin!(future);
            tokio::select! { result=&mut future => panic!("unexpected completion: {}",result.is_ok()), _=model.entered.notified()=>{} }
            if cancel {
                token.cancel();
                assert_eq!(
                    future.await.err().unwrap().error.info.category,
                    ErrorCategory::Cancelled
                );
            }
        }
        assert_eq!(model.drops.load(Ordering::SeqCst), 1);
        assert_eq!(sink.accepted.load(Ordering::SeqCst), 0);
        let seen = seen.lock().unwrap();
        check_order(&seen, &["start", "delta:partial", "returned", "end"]);
        assert_eq!(
            seen.last().unwrap().record.status,
            if cancel {
                GenerationStatus::Failed
            } else {
                GenerationStatus::Interrupted
            }
        );
    }
}
struct OptionalCost {
    estimate: Option<f64>,
    calls: AtomicUsize,
}
impl CostEstimator for OptionalCost {
    fn cost(&self, _: &str, _: &Usage) -> f64 {
        panic!("runner must preserve the optional estimate")
    }
    fn estimate_cost(&self, _: &str, _: &Usage) -> Option<f64> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.estimate
    }
}

#[tokio::test]
async fn generation_cost_distinguishes_unknown_zero_and_priced_retained_responses() {
    for estimate in [None, Some(0.0), Some(0.25)] {
        for streamed in [false, true] {
            for fail in [false, true] {
                let mut steps = vec![complete()];
                if fail {
                    steps.push(Step::Fail);
                }
                let model = Offline::new(vec![steps]);
                let seen = Arc::new(Mutex::new(vec![]));
                let sink = Arc::new(Sink::default());
                let costs = Arc::new(OptionalCost {
                    estimate,
                    calls: AtomicUsize::new(0),
                });
                let mut cfg = config(&seen, &sink);
                cfg.cost_estimator = Some(costs.clone());
                let runner = runner(&model, cfg);
                let result = if streamed {
                    runner
                        .stream(context(&CancellationToken::new()), request(), sink)
                        .finish()
                        .await
                } else {
                    runner
                        .run(context(&CancellationToken::new()), request(), sink)
                        .await
                };
                assert_eq!(result.is_err(), fail);
                let result = match result {
                    Ok(outcome) => outcome.result,
                    Err(error) => *error.partial.unwrap(),
                };
                assert_eq!(result.responses.len(), 1);
                let metrics = result.metrics.unwrap();
                assert_eq!(metrics.cost_usd, estimate.unwrap_or(0.0));
                assert_eq!(metrics.cost_known, estimate.is_some());
                assert_eq!(costs.calls.load(Ordering::SeqCst), 1);
                let seen = seen.lock().unwrap();
                let ended = seen.iter().find(|event| event.event == "end").unwrap();
                assert_eq!(ended.record.cost_usd, estimate);
                assert_eq!(
                    ended.record.status,
                    if fail {
                        GenerationStatus::Failed
                    } else {
                        GenerationStatus::Completed
                    }
                );
            }
        }
    }
}

struct NumericCost(AtomicUsize);

#[tokio::test]
async fn cumulative_cost_sums_responses_without_losing_unknown_prices() {
    struct Costs(Mutex<VecDeque<Option<f64>>>);
    impl CostEstimator for Costs {
        fn cost(&self, _: &str, _: &Usage) -> f64 {
            panic!("optional cost must be retained")
        }
        fn estimate_cost(&self, _: &str, _: &Usage) -> Option<f64> {
            self.0
                .lock()
                .unwrap()
                .pop_front()
                .expect("duplicate estimate")
        }
    }
    for estimates in [
        [Some(0.25), Some(0.75)],
        [Some(0.25), None],
        [None, Some(0.75)],
        [Some(0.0), Some(0.0)],
    ] {
        let mut first = answer();
        first.items = vec![RunItem::ToolCall {
            call: ToolCall {
                id: "missing".into(),
                name: "missing".into(),
                arguments: serde_json::json!({}),
                raw_arguments: None,
            },
        }];
        let model = Offline::new(vec![
            vec![Step::Event(ModelEvent::Complete { response: first })],
            vec![complete()],
        ]);
        let seen = Arc::new(Mutex::new(vec![]));
        let sink = Arc::new(Sink::default());
        let costs = Arc::new(Costs(Mutex::new(estimates.into())));
        let mut cfg = config(&seen, &sink);
        cfg.cost_estimator = Some(costs.clone());
        let result = runner(&model, cfg)
            .run(context(&CancellationToken::new()), request(), sink)
            .await
            .unwrap()
            .result;
        assert_eq!(result.responses.len(), 2);
        let metrics = result.metrics.unwrap();
        assert_eq!(
            metrics.cost_usd,
            estimates.into_iter().flatten().sum::<f64>()
        );
        assert_eq!(metrics.cost_known, estimates.iter().all(Option::is_some));
        assert!(costs.0.lock().unwrap().is_empty());
    }
}

impl CostEstimator for NumericCost {
    fn cost(&self, _: &str, _: &Usage) -> f64 {
        self.0.fetch_add(1, Ordering::SeqCst);
        0.5
    }
}

#[tokio::test]
async fn numeric_cost_estimators_remain_known_and_are_called_once() {
    let seen = Arc::new(Mutex::new(vec![]));
    let sink = Arc::new(Sink::default());
    let costs = Arc::new(NumericCost(AtomicUsize::new(0)));
    let mut cfg = config(&seen, &sink);
    cfg.cost_estimator = Some(costs.clone());
    cfg.blocking_reasoning = false;
    let result = runner(&Offline::new(vec![]), cfg)
        .run(context(&CancellationToken::new()), request(), sink)
        .await
        .unwrap();
    assert_eq!(result.result.metrics.unwrap().cost_usd, 0.5);
    assert_eq!(
        seen.lock().unwrap().last().unwrap().record.cost_usd,
        Some(0.5)
    );
    assert_eq!(costs.0.load(Ordering::SeqCst), 1);
}

struct Signal(Arc<tokio::sync::Notify>);

#[tokio::test]
async fn failed_stream_keeps_exact_response_cost_after_large_prior_total() {
    struct SequenceCost(Mutex<VecDeque<f64>>);
    impl CostEstimator for SequenceCost {
        fn cost(&self, _: &str, _: &Usage) -> f64 {
            self.0
                .lock()
                .unwrap()
                .pop_front()
                .expect("duplicate estimate")
        }
    }
    let mut first = answer();
    first.items = vec![RunItem::ToolCall {
        call: ToolCall {
            id: "unavailable".into(),
            name: "unavailable".into(),
            arguments: serde_json::json!({}),
            raw_arguments: None,
        },
    }];
    let model = Offline::new(vec![
        vec![Step::Event(ModelEvent::Complete { response: first })],
        vec![complete(), Step::Fail],
    ]);
    let seen = Arc::new(Mutex::new(vec![]));
    let sink = Arc::new(Sink::default());
    let costs = Arc::new(SequenceCost(Mutex::new([1e16, 0.25].into())));
    let mut cfg = config(&seen, &sink);
    cfg.cost_estimator = Some(costs.clone());
    let error = runner(&model, cfg)
        .run(context(&CancellationToken::new()), request(), sink)
        .await
        .err()
        .unwrap();
    assert_eq!(error.partial.unwrap().responses.len(), 2);
    assert!(costs.0.lock().unwrap().is_empty());
    let seen = seen.lock().unwrap();
    let last = &seen.last().unwrap().record;
    assert_eq!(last.status, GenerationStatus::Failed);
    assert_eq!(last.cost_usd, Some(0.25));
}
impl ImmediateInputSignal for Signal {
    fn wait<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, ()> {
        Box::pin(self.0.notified())
    }
}
struct Poller;
impl ImmediateInputPoller for Poller {
    fn poll<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<ImmediateInputBatch, Error>> {
        Box::pin(async { Ok(ImmediateInputBatch::default()) })
    }
}
#[tokio::test]
async fn only_surfaced_reasoning_prevents_immediate_interruption() {
    for visible in [false, true] {
        let event = if visible {
            reasoning("visible")
        } else {
            Step::Event(ModelEvent::TextDelta {
                delta: "hidden".into(),
            })
        };
        let model = Offline::new(vec![vec![event, Step::Wake, complete()], vec![complete()]]);
        let seen = Arc::new(Mutex::new(vec![]));
        let sink = Arc::new(Sink::default());
        let mut cfg = config(&seen, &sink);
        cfg.immediate_input_signal = Some(Arc::new(Signal(model.wake.clone())));
        cfg.immediate_input_poller = Some(Arc::new(Poller));
        runner(&model, cfg)
            .run(context(&CancellationToken::new()), request(), sink)
            .await
            .unwrap();
        assert_eq!(
            model.starts.load(Ordering::SeqCst),
            if visible { 1 } else { 2 }
        );
        let seen = seen.lock().unwrap();
        if visible {
            check_order(
                &seen,
                &["start", "delta:visible", "returned", "end", "accepted"],
            );
        } else {
            check_order(
                &seen,
                &[
                    "start", "returned", "end", "start", "returned", "end", "accepted",
                ],
            );
            assert_eq!(seen[4].record.status, GenerationStatus::Interrupted);
        }
    }
}
#[derive(Default)]
struct Store(Mutex<Vec<RunnerCheckpoint>>);
impl CheckpointStore for Store {
    fn persist<'a>(
        &'a self,
        _: &'a Context,
        checkpoint: &'a RunnerCheckpoint,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(checkpoint.clone());
            Ok(())
        })
    }
}
#[tokio::test]
async fn durable_replay_binds_blocking_reasoning_selection() {
    for enabled in [false, true] {
        let model = Offline::new(vec![vec![complete()]]);
        let store = Arc::new(Store::default());
        let sink = Arc::new(Sink::default());
        runner(
            &model,
            RunnerConfig {
                blocking_reasoning: enabled,
                ..Default::default()
            },
        )
        .run_durable(
            context(&CancellationToken::new()),
            request(),
            sink.clone(),
            DurableRun::new(store.clone()),
        )
        .await
        .unwrap();
        let checkpoint = store
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.execution_boundary() == "model_completed")
            .unwrap()
            .clone();
        let error = runner(
            &model,
            RunnerConfig {
                blocking_reasoning: !enabled,
                ..Default::default()
            },
        )
        .run_durable(
            context(&CancellationToken::new()),
            request(),
            sink,
            DurableRun {
                resume: Some(checkpoint),
                ..DurableRun::new(store)
            },
        )
        .await
        .err()
        .unwrap();
        assert!(
            error
                .error
                .info
                .message
                .contains("configuration or security policy changed"),
            "{}",
            error.error.info.message
        );
        assert_eq!(
            model.starts.load(Ordering::SeqCst) + model.completes.load(Ordering::SeqCst),
            1
        );
    }
}

#[tokio::test]
async fn reasoning_without_an_observer_is_hidden_and_retryable() {
    let model = Offline::new(vec![
        vec![reasoning("hidden"), Step::Fail],
        vec![complete()],
    ]);
    let sink = Arc::new(Sink::default());
    let config = RunnerConfig {
        blocking_reasoning: true,
        retry: RetryPolicy {
            max_retries: 1,
            initial_delay: Duration::ZERO,
            ..Default::default()
        },
        ..Default::default()
    };
    let outcome = runner(&model, config)
        .run(context(&CancellationToken::new()), request(), sink.clone())
        .await
        .unwrap();
    assert_eq!(model.starts.load(Ordering::SeqCst), 2);
    assert_eq!(outcome.result.new_items, answer().items);
    assert!(!sink.events.lock().unwrap().iter().any(|event| matches!(
        event,
        RunEvent::Model {
            event: ModelEvent::ReasoningDelta { .. }
        }
    )));
}

#[tokio::test(start_paused = true)]
async fn blocking_reasoning_stream_remains_idle_bounded() {
    let model = Offline::new(vec![vec![reasoning("partial"), Step::Wait]]);
    let seen = Arc::new(Mutex::new(vec![]));
    let sink = Arc::new(Sink::default());
    let mut cfg = config(&seen, &sink);
    cfg.model_idle_timeout = Some(Duration::from_secs(1));
    let error = runner(&model, cfg)
        .run(context(&CancellationToken::new()), request(), sink.clone())
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::DeadlineExceeded);
    assert_eq!(model.starts.load(Ordering::SeqCst), 1);
    assert_eq!(model.drops.load(Ordering::SeqCst), 1);
    assert_eq!(sink.accepted.load(Ordering::SeqCst), 0);
    let seen = seen.lock().unwrap();
    check_order(&seen, &["start", "delta:partial", "returned", "end"]);
    assert_eq!(seen.last().unwrap().record.status, GenerationStatus::Failed);
}
