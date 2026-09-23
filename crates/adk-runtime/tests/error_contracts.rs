use adk_core::*;
use adk_runtime::*;
use serde_json::json;
use std::{
    collections::VecDeque,
    num::NonZeroU32,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

fn message(role: Role, text: &str) -> RunItem {
    RunItem::Message {
        message: Message {
            role,
            content: vec![Content::Text { text: text.into() }],
        },
    }
}
fn call() -> RunItem {
    RunItem::ToolCall {
        call: ToolCall {
            id: "call-1".into(),
            name: "work".into(),
            arguments: json!({}),
        },
    }
}
fn response(items: Vec<RunItem>) -> ModelResponse {
    ModelResponse {
        snapshot_raw: None,
        raw: None,
        items,
        usage: Usage {
            input_tokens: 3,
            output_tokens: 2,
            ..Usage::default()
        },
        end_turn: None,
        response_id: None,
        metadata: Default::default(),
    }
}
fn complete(items: Vec<RunItem>) -> Step {
    Step::Event(ModelEvent::Complete {
        response: response(items),
    })
}
fn answer() -> Step {
    complete(vec![message(Role::Assistant, "done")])
}
fn request(turns: u32) -> RunRequest {
    RunRequest {
        input_provenance: Vec::new(),
        input: vec![message(Role::User, "go")],
        policy: RunPolicy {
            max_turns: NonZeroU32::new(turns).unwrap(),
            ..RunPolicy::default()
        },
    }
}
fn context() -> Context {
    Context {
        run_id: "error-contract".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn provider_error() -> Error {
    Error::new(ErrorCategory::Provider, "provider broke")
}

enum Step {
    Event(ModelEvent),
    Error,
    Pending,
}
struct Closed(Arc<AtomicUsize>);
impl Drop for Closed {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
struct Script {
    scripts: Mutex<VecDeque<VecDeque<Step>>>,
    calls: AtomicUsize,
    closed: Arc<AtomicUsize>,
    fallback_advice: bool,
}
impl Script {
    fn new(scripts: Vec<Vec<Step>>) -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(scripts.into_iter().map(Into::into).collect()),
            calls: AtomicUsize::new(0),
            closed: Arc::new(AtomicUsize::new(0)),
            fallback_advice: false,
        })
    }
    fn take(&self) -> VecDeque<Step> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.scripts
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected model dispatch")
    }
}
impl Model for Script {
    fn retry_advice(&self, _: &Error) -> Option<ModelRetryAdvice> {
        self.fallback_advice.then(|| ModelRetryAdvice {
            should_retry: true,
            retry_after: Duration::ZERO,
            reason: "overloaded".into(),
        })
    }
    fn provider(&self) -> &str {
        "error-contract"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            let _closed = Closed(self.closed.clone());
            match self.take().pop_front().unwrap() {
                Step::Event(ModelEvent::Complete { response }) => Ok(response),
                Step::Error => Err(provider_error()),
                Step::Pending => std::future::pending().await,
                _ => panic!("complete model requires Complete"),
            }
        })
    }
}
struct Stream {
    steps: VecDeque<Step>,
    _closed: Closed,
}
impl ModelStream for Stream {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async move {
            match self.steps.pop_front() {
                Some(Step::Event(event)) => Ok(Some(event)),
                Some(Step::Error) => Err(provider_error()),
                Some(Step::Pending) => std::future::pending().await,
                None => Ok(None),
            }
        })
    }
}
impl StreamingModel for Script {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            Ok(Box::new(Stream {
                steps: self.take(),
                _closed: Closed(self.closed.clone()),
            }) as Box<dyn ModelStream>)
        })
    }
}

#[derive(Debug)]
enum Record {
    Event(RunEvent),
    Hook(Observation),
}
#[derive(Default)]
struct Recorder {
    records: Mutex<Vec<Record>>,
    reject_raw: bool,
    reject_failed: bool,
}
impl Host for Recorder {
    fn emit<'a>(&'a self, _: &'a Context, event: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let reject = self.reject_failed && matches!(event, RunEvent::Failed { .. });
            self.records.lock().unwrap().push(Record::Event(event));
            if reject {
                Err(Error::new(ErrorCategory::Host, "failed sink broke"))
            } else {
                Ok(())
            }
        })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { Ok(ApprovalDecision::Approve) })
    }
}
impl RunHooks for Recorder {
    fn observe<'a>(
        &'a self,
        _: &'a Context,
        observation: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let reject =
                self.reject_raw && matches!(observation, Observation::RawToolOutput { .. });
            self.records.lock().unwrap().push(Record::Hook(observation));
            if reject {
                Err(Error::new(
                    ErrorCategory::Guardrail,
                    "security hook rejected",
                ))
            } else {
                Ok(())
            }
        })
    }
}
fn agent(model: Arc<Script>) -> AgentConfig {
    AgentConfig::new("contract", ModelBinding::streaming("primary", model))
}
fn config(recorder: &Arc<Recorder>) -> RunnerConfig {
    RunnerConfig {
        hooks: Some(recorder.clone()),
        ..RunnerConfig::default()
    }
}
fn partial(error: RunError, category: ErrorCategory) -> RunResult {
    assert_eq!(error.error.info.category, category);
    let partial = *error.partial.expect("native diagnostic snapshot");
    assert_eq!(partial.status, RunStatus::Incomplete);
    assert!(partial.final_output.is_none());
    partial
}
fn no_acceptance(recorder: &Recorder) {
    assert!(
        !recorder
            .records
            .lock()
            .unwrap()
            .iter()
            .any(|r| matches!(r, Record::Hook(Observation::ModelAccepted { .. })))
    );
}

#[tokio::test]
async fn invalid_configuration_and_input_never_dispatch() {
    let model = Script::new(vec![]);
    let invalid = RunnerConfig {
        limits: Limits {
            max_cost: Some(1.0),
            max_tokens: None,
        },
        ..RunnerConfig::default()
    };
    assert_eq!(
        Runner::new(agent(model.clone()), invalid)
            .err()
            .unwrap()
            .info
            .category,
        ErrorCategory::InvalidInput
    );
    for streaming in [false, true] {
        let recorder = Arc::new(Recorder::default());
        let runner = Runner::new(agent(model.clone()), config(&recorder)).unwrap();
        let mut req = request(1);
        req.input.push(call());
        let result = if streaming {
            runner
                .stream(context(), req, recorder.clone())
                .finish()
                .await
        } else {
            runner.run(context(), req, recorder.clone()).await
        };
        let snapshot = partial(result.err().unwrap(), ErrorCategory::InvalidInput);
        assert!(snapshot.responses.is_empty());
        assert!(snapshot.new_items.is_empty());
        assert!(!recorder.records.lock().unwrap().iter().any(|r| matches!(
            r,
            Record::Event(RunEvent::Started { .. } | RunEvent::ToolStarted { .. })
                | Record::Hook(Observation::ModelAttempt { .. })
        )));
    }
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn provider_failure_keeps_input_only_and_failed_sink_cannot_replace_error() {
    for streaming in [false, true] {
        let model = Script::new(vec![vec![Step::Error]]);
        let recorder = Arc::new(Recorder {
            reject_failed: true,
            ..Recorder::default()
        });
        let runner = Runner::new(agent(model), config(&recorder)).unwrap();
        let result = if streaming {
            runner
                .stream(context(), request(1), recorder.clone())
                .finish()
                .await
        } else {
            runner.run(context(), request(1), recorder.clone()).await
        };
        let error = result.err().unwrap();
        assert_eq!(error.error.info.message, "provider broke");
        let snapshot = partial(error, ErrorCategory::Provider);
        assert_eq!(snapshot.history, request(1).input);
        assert!(snapshot.new_items.is_empty());
        assert!(snapshot.responses.is_empty());
        assert_eq!(snapshot.usage, Usage::default());
        no_acceptance(&recorder);
        assert!(recorder.records.lock().unwrap().iter().any(|r| matches!(r, Record::Event(RunEvent::Failed { error }) if error.category == ErrorCategory::Provider)));
    }
}

#[tokio::test]
async fn delta_or_complete_then_error_is_diagnostic_not_model_acceptance() {
    for completed in [false, true] {
        let item = message(Role::Assistant, "partial");
        let first = if completed {
            complete(vec![item.clone()])
        } else {
            Step::Event(ModelEvent::TextDelta {
                delta: "partial".into(),
            })
        };
        let model = Script::new(vec![vec![first, Step::Error], vec![answer()]]);
        let recorder = Arc::new(Recorder::default());
        let mut cfg = config(&recorder);
        cfg.retry = retry();
        let runner = Runner::new(agent(model.clone()), cfg).unwrap();
        let mut input = request(3);
        input.input_provenance = vec![ItemProvenance::Unattributed];
        let snapshot = partial(
            runner
                .stream(context(), input, recorder.clone())
                .finish()
                .await
                .err()
                .unwrap(),
            ErrorCategory::Provider,
        );
        assert_eq!(snapshot.new_items, vec![item.clone()]);
        assert_eq!(snapshot.history, vec![message(Role::User, "go"), item]);
        let generated = ItemProvenance::Agent {
            name: "contract".into(),
        };
        assert_eq!(snapshot.new_items_provenance, vec![generated.clone()]);
        assert_eq!(
            snapshot.history_provenance,
            vec![ItemProvenance::Unattributed, generated]
        );
        assert_eq!(snapshot.responses.len(), usize::from(completed));
        assert_eq!(snapshot.usage.input_tokens, if completed { 3 } else { 0 });
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
        assert_eq!(model.closed.load(Ordering::SeqCst), 1);
        no_acceptance(&recorder);
        let records = recorder.records.lock().unwrap();
        assert!(
            !records
                .iter()
                .any(|r| matches!(r, Record::Hook(Observation::Usage { .. })))
        );
        assert_eq!(
            records
                .iter()
                .filter(|r| matches!(
                    r,
                    Record::Event(RunEvent::Model {
                        event: ModelEvent::Complete { .. }
                    })
                ))
                .count(),
            usize::from(completed)
        );
    }
}

struct Work {
    definition: ToolDefinition,
    mode: u8,
    calls: AtomicUsize,
    closed: Arc<AtomicUsize>,
}
impl Work {
    fn new(mode: u8) -> Arc<Self> {
        Arc::new(Self {
            definition: ToolDefinition {
                name: "work".into(),
                description: "test work".into(),
                input_schema: schemars::json_schema!({"type":"object"}),
                read_only: true,
                requires_approval: false,
            },
            mode,
            calls: AtomicUsize::new(0),
            closed: Arc::new(AtomicUsize::new(0)),
        })
    }
}
impl Tool for Work {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let _closed = Closed(self.closed.clone());
            match self.mode {
                1 => Err(Error::new(ErrorCategory::Tool, "ordinary tool failure")),
                2 => std::future::pending().await,
                _ => Ok(ToolOutput {
                    content: vec![Content::Text {
                        text: "sensitive raw output".into(),
                    }],
                    is_error: false,
                    should_pause: false,
                }),
            }
        })
    }
}

#[tokio::test]
async fn accepted_response_precedes_usage_and_tool_start_only_after_successful_eof() {
    for streaming in [false, true] {
        let model = Script::new(vec![vec![complete(vec![call()])], vec![answer()]]);
        let tool = Work::new(0);
        let mut a = agent(model);
        a.tools.push(tool);
        let recorder = Arc::new(Recorder::default());
        let runner = Runner::new(a, config(&recorder)).unwrap();
        let outcome = if streaming {
            runner
                .stream(context(), request(2), recorder.clone())
                .finish()
                .await
        } else {
            runner.run(context(), request(2), recorder.clone()).await
        }
        .unwrap();
        assert_eq!(outcome.result.status, RunStatus::Completed);
        let records = recorder.records.lock().unwrap();
        let accepted = records.iter().position(|r| matches!(r, Record::Hook(Observation::ModelAccepted { agent, response }) if agent == "contract" && response.items == vec![call()])).unwrap();
        let usage = records
            .iter()
            .position(|r| matches!(r, Record::Hook(Observation::Usage { .. })))
            .unwrap();
        let started = records
            .iter()
            .position(|r| matches!(r, Record::Event(RunEvent::ToolStarted { .. })))
            .unwrap();
        assert!(accepted < usage && usage < started);
        assert_eq!(
            records
                .iter()
                .filter(|r| matches!(r, Record::Hook(Observation::ModelAccepted { .. })))
                .count(),
            2
        );
        if streaming {
            let raw = records
                .iter()
                .position(|r| {
                    matches!(
                        r,
                        Record::Event(RunEvent::Model {
                            event: ModelEvent::Complete { .. }
                        })
                    )
                })
                .unwrap();
            assert!(raw < accepted);
        }
    }
}

#[tokio::test]
async fn post_tool_security_hook_is_fatal_and_suppresses_successful_finish() {
    let model = Script::new(vec![vec![complete(vec![call()])]]);
    let tool = Work::new(0);
    let mut a = agent(model.clone());
    a.tools.push(tool.clone());
    let recorder = Arc::new(Recorder {
        reject_raw: true,
        ..Recorder::default()
    });
    let snapshot = partial(
        Runner::new(a, config(&recorder))
            .unwrap()
            .run(context(), request(2), recorder.clone())
            .await
            .err()
            .unwrap(),
        ErrorCategory::Guardrail,
    );
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
    assert!(
        matches!(&snapshot.new_items[1], RunItem::ToolResult { call_id, output } if call_id == "call-1" && output.is_error)
    );
    assert!(!format!("{:?}", snapshot.history).contains("sensitive raw output"));
    assert!(!recorder.records.lock().unwrap().iter().any(|r| matches!(
        r,
        Record::Event(RunEvent::ToolFinished { .. } | RunEvent::Finished { .. })
    )));
}

#[tokio::test]
async fn ordinary_failure_timeout_unknown_and_inaccessible_tools_pair_and_continue() {
    for mode in 1..=4 {
        let model = Script::new(vec![vec![complete(vec![call()])], vec![answer()]]);
        let tool = Work::new(mode);
        let mut a = agent(model.clone());
        if mode != 3 {
            a.tools.push(tool.clone());
        }
        let recorder = Arc::new(Recorder::default());
        let mut req = request(2);
        req.policy.tools.timeout = Some(Duration::from_millis(5));
        if mode == 4 {
            req.policy.tools.denied_tools.insert("work".into());
        }
        let result = Runner::new(a, config(&recorder))
            .unwrap()
            .run(context(), req, recorder.clone())
            .await
            .unwrap()
            .result;
        assert_eq!(result.status, RunStatus::Completed);
        assert_eq!(model.calls.load(Ordering::SeqCst), 2);
        assert_eq!(tool.calls.load(Ordering::SeqCst), usize::from(mode < 3));
        assert!(
            matches!(&result.new_items[1], RunItem::ToolResult { call_id, output } if call_id == "call-1" && output.is_error)
        );
        if mode >= 3 {
            assert!(
                matches!(&result.new_items[1], RunItem::ToolResult { output, .. }
                if output.content.iter().any(|content| matches!(content, Content::Text { text } if text.contains("unknown tool: work"))))
            );
        }
        let records = recorder.records.lock().unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|r| matches!(r, Record::Hook(Observation::RawToolOutput { .. })))
                .count(),
            usize::from(mode < 3)
        );
        assert_eq!(
            records
                .iter()
                .filter(|r| matches!(r, Record::Event(RunEvent::ToolStarted { .. })))
                .count(),
            usize::from(mode < 3)
        );
        assert!(
            !records
                .iter()
                .any(|r| matches!(r, Record::Event(RunEvent::Failed { .. })))
        );
    }
}

fn retry() -> RetryPolicy {
    RetryPolicy {
        max_retries: 1,
        initial_delay: Duration::ZERO,
        max_delay: Duration::ZERO,
        retryable: |_| true,
    }
}
struct Handler(bool);
impl ModelErrorHandler for Handler {
    fn handle(&self, _: &str, _: u32, _: &Error) -> ModelErrorAction {
        if self.0 {
            ModelErrorAction::Retry
        } else {
            ModelErrorAction::Continue
        }
    }
}
#[tokio::test]
async fn retry_fallback_and_error_handler_attempts_all_consume_turns() {
    for mode in 0..4 {
        let mut model = Script::new(vec![vec![Step::Error], vec![answer()]]);
        Arc::get_mut(&mut model).unwrap().fallback_advice = mode == 1;
        let fallback = Script::new(vec![vec![answer()]]);
        let mut a = agent(model.clone());
        if mode == 1 {
            a.fallbacks
                .push(ModelBinding::complete("fallback", fallback.clone()));
        }
        let recorder = Arc::new(Recorder::default());
        let mut cfg = config(&recorder);
        cfg.retry = retry();
        if mode >= 2 {
            cfg.error_handler = Some(Arc::new(Handler(mode == 2)));
        }
        let snapshot = partial(
            Runner::new(a, cfg)
                .unwrap()
                .run(context(), request(1), recorder.clone())
                .await
                .err()
                .unwrap(),
            ErrorCategory::MaxTurns,
        );
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback.calls.load(Ordering::SeqCst), 0);
        assert!(snapshot.responses.is_empty());
        no_acceptance(&recorder);
        assert_eq!(
            recorder
                .records
                .lock()
                .unwrap()
                .iter()
                .any(|r| matches!(r, Record::Hook(Observation::Fallback { .. }))),
            mode == 1
        );
    }
}

#[tokio::test]
async fn local_idle_before_output_can_retry_but_visible_output_and_parent_deadline_cannot() {
    for mode in 0..3 {
        let steps = if mode == 1 {
            vec![
                Step::Event(ModelEvent::TextDelta {
                    delta: "visible".into(),
                }),
                Step::Pending,
            ]
        } else {
            vec![Step::Pending]
        };
        let model = Script::new(vec![steps, vec![answer()]]);
        let recorder = Arc::new(Recorder::default());
        let mut cfg = config(&recorder);
        cfg.retry = retry();
        cfg.model_idle_timeout = Some(Duration::from_millis(if mode == 2 { 1000 } else { 5 }));
        if mode == 0 {
            cfg.retry.retryable = RetryPolicy::default().retryable;
        }
        let mut ctx = context();
        if mode == 2 {
            ctx.deadline = Some(Instant::now() + Duration::from_millis(5));
        }
        let outcome = Runner::new(agent(model.clone()), cfg)
            .unwrap()
            .stream(ctx, request(3), recorder.clone())
            .finish()
            .await;
        if mode == 0 {
            assert_eq!(outcome.unwrap().result.status, RunStatus::Completed);
            assert_eq!(model.calls.load(Ordering::SeqCst), 2);
        } else {
            partial(outcome.err().unwrap(), ErrorCategory::DeadlineExceeded);
            assert_eq!(model.calls.load(Ordering::SeqCst), 1);
            no_acceptance(&recorder);
        }
        assert_eq!(
            model.closed.load(Ordering::SeqCst),
            model.calls.load(Ordering::SeqCst)
        );
    }
}

#[tokio::test]
async fn parent_cancellation_and_owner_drop_close_pending_model_and_tool_resources() {
    for tool_pending in [false, true] {
        for cancel in [false, true] {
            let model = Script::new(vec![if tool_pending {
                vec![complete(vec![call()])]
            } else {
                vec![Step::Pending]
            }]);
            let tool = Work::new(2);
            let mut a = agent(model.clone());
            a.tools.push(tool.clone());
            let recorder = Arc::new(Recorder::default());
            let mut cfg = config(&recorder);
            cfg.retry = retry();
            cfg.model_idle_timeout = None;
            let mut ctx = context();
            let cancellation = Arc::new(CancellationToken::new());
            ctx.cancellation = cancellation.clone();
            let runner = Runner::new(a, cfg).unwrap();
            let mut stream = runner.stream(ctx, request(3), recorder.clone());
            assert!(
                tokio::time::timeout(Duration::from_millis(10), async {
                    while stream.next().await.is_some() {}
                })
                .await
                .is_err()
            );
            assert_eq!(model.calls.load(Ordering::SeqCst), 1);
            assert_eq!(tool.calls.load(Ordering::SeqCst), usize::from(tool_pending));
            if cancel {
                cancellation.cancel();
                let error = tokio::time::timeout(Duration::from_secs(1), stream.finish())
                    .await
                    .unwrap()
                    .err()
                    .unwrap();
                partial(error, ErrorCategory::Cancelled);
            } else {
                drop(stream);
            }
            assert_eq!(model.closed.load(Ordering::SeqCst), 1);
            assert_eq!(
                tool.closed.load(Ordering::SeqCst),
                usize::from(tool_pending)
            );
            assert!(!recorder.records.lock().unwrap().iter().any(|r| matches!(
                r,
                Record::Event(RunEvent::ToolFinished { .. } | RunEvent::Finished { .. })
            )));
        }
    }
}

struct UnitCost;
impl CostEstimator for UnitCost {
    fn cost(&self, _: &str, _: &Usage) -> f64 {
        1.0
    }
}
#[tokio::test]
async fn zero_and_equal_token_or_cost_limits_stop_before_tool_effects() {
    for cost in [false, true] {
        for zero in [false, true] {
            let model = Script::new(vec![vec![complete(vec![call()])]]);
            let tool = Work::new(0);
            let mut a = agent(model.clone());
            a.tools.push(tool.clone());
            let recorder = Arc::new(Recorder::default());
            let mut cfg = config(&recorder);
            cfg.cost_estimator = Some(Arc::new(UnitCost));
            cfg.limits = if cost {
                Limits {
                    max_cost: Some(if zero { 0.0 } else { 1.0 }),
                    max_tokens: None,
                }
            } else {
                Limits {
                    max_tokens: Some(if zero { 0 } else { 5 }),
                    max_cost: None,
                }
            };
            let snapshot = partial(
                Runner::new(a, cfg)
                    .unwrap()
                    .run(context(), request(1), recorder.clone())
                    .await
                    .err()
                    .unwrap(),
                ErrorCategory::Guardrail,
            );
            assert_eq!(model.calls.load(Ordering::SeqCst), usize::from(!zero));
            assert_eq!(snapshot.responses.len(), usize::from(!zero));
            assert_eq!(snapshot.usage.input_tokens, if zero { 0 } else { 3 });
            assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
            assert!(
                !recorder
                    .records
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|r| matches!(r, Record::Event(RunEvent::ToolStarted { .. })))
            );
        }
    }
}
