use adk_core::*;
use adk_runtime::*;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    num::NonZeroU32,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

fn message(role: Role, value: &str) -> RunItem {
    RunItem::Message {
        message: Message {
            role,
            content: vec![Content::Text { text: value.into() }],
        },
    }
}
fn response(items: Vec<RunItem>, end_turn: Option<bool>) -> ModelResponse {
    ModelResponse {
        items,
        usage: Usage {
            input_tokens: 10,
            output_tokens: 2,
            context_tokens: Some(12),
            ..Usage::default()
        },
        end_turn,
        response_id: None,
        metadata: Default::default(),
    }
}
fn answer(value: &str) -> ModelResponse {
    response(vec![message(Role::Assistant, value)], None)
}
fn call(id: &str, name: &str) -> RunItem {
    RunItem::ToolCall {
        call: ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: json!({}),
        },
    }
}
fn policy(turns: u32) -> RunPolicy {
    RunPolicy {
        max_turns: NonZeroU32::new(turns).unwrap(),
        tools: ToolPolicy::default(),
        tool_use: ToolUseBehavior::Continue,
    }
}
fn request(turns: u32) -> RunRequest {
    RunRequest {
        input: vec![message(Role::User, "go")],
        policy: policy(turns),
    }
}
fn context() -> Context {
    Context {
        run_id: "run-1".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn provider_error() -> Error {
    Error::new(ErrorCategory::Provider, "retryable provider failure")
}

#[derive(Default)]
struct TestHost {
    events: Mutex<Vec<RunEvent>>,
    approvals: Mutex<VecDeque<ApprovalDecision>>,
}
impl Host for TestHost {
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
        Box::pin(async move {
            Ok(self
                .approvals
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(ApprovalDecision::Approve))
        })
    }
}

enum StreamStep {
    Event(ModelEvent),
    Fail,
    Pending,
}
struct TestStream {
    steps: VecDeque<StreamStep>,
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}
impl ModelStream for TestStream {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async move {
            self.polls.fetch_add(1, Ordering::SeqCst);
            match self.steps.pop_front() {
                Some(StreamStep::Event(event)) => Ok(Some(event)),
                Some(StreamStep::Fail) => Err(provider_error()),
                Some(StreamStep::Pending) => std::future::pending().await,
                None => Ok(None),
            }
        })
    }
}
impl Drop for TestStream {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
#[derive(Default)]
struct TestModel {
    replies: Mutex<VecDeque<Result<ModelResponse, Error>>>,
    streams: Mutex<VecDeque<Vec<StreamStep>>>,
    requests: Mutex<Vec<ModelRequest>>,
    completes: AtomicUsize,
    starts: AtomicUsize,
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}
impl TestModel {
    fn with(replies: Vec<Result<ModelResponse, Error>>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into()),
            ..Self::default()
        })
    }
    fn streaming(steps: Vec<StreamStep>) -> Arc<Self> {
        Arc::new(Self {
            streams: Mutex::new(vec![steps].into()),
            ..Self::default()
        })
    }
}
impl Model for TestModel {
    fn provider(&self) -> &str {
        "test"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            self.completes.fetch_add(1, Ordering::SeqCst);
            self.requests.lock().unwrap().push(request);
            let result = self.replies.lock().unwrap().pop_front();
            match result {
                Some(result) => result,
                None => std::future::pending().await,
            }
        })
    }
}
impl StreamingModel for TestModel {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            self.starts.fetch_add(1, Ordering::SeqCst);
            self.requests.lock().unwrap().push(request);
            let steps = self
                .streams
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_default()
                .into();
            Ok(Box::new(TestStream {
                steps,
                polls: self.polls.clone(),
                drops: self.drops.clone(),
            }) as Box<dyn ModelStream>)
        })
    }
}

struct TestTool {
    definition: ToolDefinition,
    output: ToolOutput,
    calls: AtomicUsize,
    pending: bool,
}
impl TestTool {
    fn new(name: &str, approval: bool, pause: bool) -> Arc<Self> {
        Arc::new(Self {
            definition: ToolDefinition {
                name: name.into(),
                description: name.into(),
                input_schema: schemars::json_schema!({"type":"object", "additionalProperties":false}),
                read_only: true,
                requires_approval: approval,
            },
            output: ToolOutput {
                content: vec![Content::Text {
                    text: format!("raw {name}"),
                }],
                is_error: false,
                should_pause: pause,
            },
            calls: AtomicUsize::new(0),
            pending: false,
        })
    }
}
impl Tool for TestTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            assert!(context.idempotency_key.is_some());
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.pending {
                std::future::pending().await
            } else {
                Ok(self.output.clone())
            }
        })
    }
}
fn agent(model: Arc<TestModel>) -> AgentConfig {
    AgentConfig::new("test", ModelBinding::streaming("primary", model))
}
fn runner(agent: AgentConfig) -> Runner {
    Runner::new(agent, RunnerConfig::default()).unwrap()
}

#[tokio::test]
async fn approval_resume_keeps_cursor_and_completed_effects() {
    let model = TestModel::with(vec![
        Ok(response(
            vec![call("1", "one"), call("2", "two"), call("3", "three")],
            Some(true),
        )),
        Ok(answer("done")),
    ]);
    let one = TestTool::new("one", false, false);
    let two = TestTool::new("two", true, false);
    let three = TestTool::new("three", false, false);
    let mut a = agent(model.clone());
    a.tools = vec![one.clone(), two.clone(), three.clone()];
    let host = Arc::new(TestHost::default());
    host.approvals
        .lock()
        .unwrap()
        .push_back(ApprovalDecision::Defer);
    let mut paused = runner(a)
        .run(context(), request(3), host.clone())
        .await
        .unwrap();
    assert_eq!(paused.result.status, RunStatus::Paused);
    assert_eq!(paused.result.pending_approvals[0].call.id, "2");
    assert_eq!(one.calls.load(Ordering::SeqCst), 1);
    assert_eq!(two.calls.load(Ordering::SeqCst), 0);
    let continuation = paused.continuation.take().unwrap();
    drop(paused);
    let done = continuation
        .resume(Some(ApprovalDecision::Approve))
        .await
        .unwrap();
    assert_eq!(done.result.final_output, Some(json!("done")));
    for t in [one, two, three] {
        assert_eq!(t.calls.load(Ordering::SeqCst), 1);
    }
    assert_eq!(model.completes.load(Ordering::SeqCst), 2);
    assert_eq!(done.result.usage.input_tokens, 20);
    let ids: Vec<_> = done
        .result
        .history
        .iter()
        .filter_map(|i| {
            if let RunItem::ToolResult { call_id, .. } = i {
                Some(call_id.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(ids, ["1", "2", "3"]);
    assert!(done.result.pending_approvals.is_empty());
}

#[tokio::test]
async fn tool_pause_resumes_next_turn_and_stop_executes_batch() {
    let model = TestModel::with(vec![
        Ok(response(vec![call("1", "one"), call("2", "two")], None)),
        Ok(answer("done")),
    ]);
    let one = TestTool::new("one", false, true);
    let two = TestTool::new("two", false, false);
    let mut a = agent(model);
    a.tools = vec![one.clone(), two.clone()];
    let paused = runner(a)
        .run(context(), request(3), Arc::new(TestHost::default()))
        .await
        .unwrap();
    assert_eq!(paused.result.status, RunStatus::Paused);
    assert_eq!(two.calls.load(Ordering::SeqCst), 1);
    let done = paused.continuation.unwrap().resume(None).await.unwrap();
    assert_eq!(done.result.status, RunStatus::Completed);
    assert_eq!(one.calls.load(Ordering::SeqCst), 1);
    let model = TestModel::with(vec![Ok(response(
        vec![call("1", "one"), call("2", "two")],
        None,
    ))]);
    let mut a = agent(model);
    a.tools = vec![one.clone(), two.clone()];
    let mut req = request(1);
    req.policy.tool_use = ToolUseBehavior::StopAfterTool;
    let result = runner(a)
        .run(context(), req, Arc::new(TestHost::default()))
        .await
        .unwrap();
    assert_eq!(result.result.status, RunStatus::Completed);
    assert!(result.result.final_output.is_none());
    assert_eq!(one.calls.load(Ordering::SeqCst), 2);
    assert_eq!(two.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn stream_is_lazy_bounded_and_drop_drops_provider() {
    let steps = (0..100)
        .map(|_| StreamStep::Event(ModelEvent::TextDelta { delta: "x".into() }))
        .collect();
    let model = TestModel::streaming(steps);
    let mut stream =
        runner(agent(model.clone())).stream(context(), request(2), Arc::new(TestHost::default()));
    assert_eq!(model.starts.load(Ordering::SeqCst), 0);
    assert!(matches!(
        stream.next().await,
        Some(RunEvent::Started { .. })
    ));
    assert!(matches!(
        stream.next().await,
        Some(RunEvent::Model {
            event: ModelEvent::TextDelta { .. }
        })
    ));
    let polls = model.polls.load(Ordering::SeqCst);
    assert!(polls <= 3, "bounded pull polled {polls} provider events");
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(model.polls.load(Ordering::SeqCst), polls);
    drop(stream);
    assert_eq!(model.drops.load(Ordering::SeqCst), 1);
    assert_eq!(model.completes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stream_emits_complete_and_failure_and_never_retries_visible_output() {
    let model = TestModel::streaming(vec![
        StreamStep::Event(ModelEvent::TextDelta {
            delta: "partial".into(),
        }),
        StreamStep::Fail,
    ]);
    let mut config = RunnerConfig::default();
    config.retry.max_retries = 2;
    let r = Runner::new(agent(model.clone()), config).unwrap();
    let mut stream = r.stream(context(), request(2), Arc::new(TestHost::default()));
    let mut failed = false;
    while let Some(event) = stream.next().await {
        failed |= matches!(event, RunEvent::Failed { .. });
    }
    assert!(failed);
    let error = stream.finish().await.err().unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::Provider);
    assert!(
        error
            .partial
            .unwrap()
            .history
            .contains(&message(Role::Assistant, "partial"))
    );
    assert_eq!(model.starts.load(Ordering::SeqCst), 1);
    let model = TestModel::streaming(vec![StreamStep::Event(ModelEvent::Complete {
        response: answer("ok"),
    })]);
    let result = runner(agent(model.clone()))
        .stream(context(), request(2), Arc::new(TestHost::default()))
        .finish()
        .await
        .unwrap();
    assert_eq!(result.result.final_output, Some(json!("ok")));
    assert_eq!(model.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn invalid_stream_protocol_is_not_success() {
    for steps in [
        vec![],
        vec![
            StreamStep::Event(ModelEvent::Complete {
                response: answer("one"),
            }),
            StreamStep::Event(ModelEvent::Complete {
                response: answer("two"),
            }),
        ],
    ] {
        let model = TestModel::streaming(steps);
        let error = runner(agent(model))
            .stream(context(), request(2), Arc::new(TestHost::default()))
            .finish()
            .await
            .err()
            .unwrap();
        assert_eq!(error.error.info.category, ErrorCategory::ModelBehavior);
    }
}

#[tokio::test]
async fn cancelled_next_future_is_safe_and_owner_drop_cleans_pending_stream() {
    let model = TestModel::streaming(vec![StreamStep::Pending]);
    let mut stream =
        runner(agent(model.clone())).stream(context(), request(2), Arc::new(TestHost::default()));
    assert!(stream.next().await.is_some());
    assert!(
        tokio::time::timeout(Duration::from_millis(5), stream.next())
            .await
            .is_err()
    );
    assert_eq!(model.polls.load(Ordering::SeqCst), 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(5), stream.next())
            .await
            .is_err()
    );
    assert_eq!(model.polls.load(Ordering::SeqCst), 1);
    drop(stream);
    assert_eq!(model.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn fallback_precedes_policy_retries_without_spending_extra_turns() {
    let primary = TestModel::with(vec![Err(provider_error())]);
    let fallback = TestModel::with(vec![Err(provider_error()), Ok(answer("fallback"))]);
    let mut a = agent(primary.clone());
    a.fallbacks = vec![ModelBinding::complete("backup", fallback.clone())];
    let mut config = RunnerConfig::default();
    config.retry.max_retries = 1;
    config.retry.initial_delay = Duration::ZERO;
    let result = Runner::new(a, config)
        .unwrap()
        .run(context(), request(1), Arc::new(TestHost::default()))
        .await
        .unwrap();
    assert_eq!(result.result.final_output, Some(json!("fallback")));
    assert_eq!(primary.completes.load(Ordering::SeqCst), 1);
    assert_eq!(fallback.completes.load(Ordering::SeqCst), 2);
    assert_eq!(fallback.requests.lock().unwrap()[0].model, "backup");
    assert_eq!(result.result.responses.len(), 1);
}

#[tokio::test]
async fn cancellation_deadline_idle_and_tool_timeout_interrupt_pending_work() {
    let cancellation = CancellationToken::new();
    let signal = cancellation.clone();
    let mut ctx = context();
    ctx.cancellation = Arc::new(cancellation);
    let r = runner(agent(TestModel::with(vec![])));
    let (result, ()) = tokio::join!(
        r.run(ctx, request(2), Arc::new(TestHost::default())),
        async {
            tokio::time::sleep(Duration::from_millis(5)).await;
            signal.cancel();
        }
    );
    assert_eq!(
        result.err().unwrap().error.info.category,
        ErrorCategory::Cancelled
    );
    let mut ctx = context();
    ctx.deadline = Some(Instant::now() + Duration::from_millis(5));
    let error = r
        .run(ctx, request(2), Arc::new(TestHost::default()))
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::DeadlineExceeded);
    let model = TestModel::streaming(vec![StreamStep::Pending]);
    let config = RunnerConfig {
        model_idle_timeout: Some(Duration::from_millis(5)),
        ..RunnerConfig::default()
    };
    let error = Runner::new(agent(model.clone()), config)
        .unwrap()
        .stream(context(), request(1), Arc::new(TestHost::default()))
        .finish()
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::DeadlineExceeded);
    assert_eq!(model.drops.load(Ordering::SeqCst), 1);
    let mut tool = TestTool::new("wait", false, false);
    Arc::get_mut(&mut tool).unwrap().pending = true;
    let mut a = agent(TestModel::with(vec![Ok(response(
        vec![call("1", "wait")],
        None,
    ))]));
    a.tools = vec![tool];
    let mut req = request(2);
    req.policy.tools.timeout = Some(Duration::from_millis(5));
    let error = runner(a)
        .run(context(), req, Arc::new(TestHost::default()))
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::DeadlineExceeded);
    assert_eq!(error.partial.unwrap().responses.len(), 1);
}

struct UnitCost;
impl CostEstimator for UnitCost {
    fn cost(&self, _: &str, _: &Usage) -> f64 {
        1.0
    }
}
#[tokio::test]
async fn turn_token_and_cost_limits_keep_partial_usage() {
    let r = runner(agent(TestModel::with(vec![Ok(response(
        vec![message(Role::Assistant, "continue")],
        Some(false),
    ))])));
    let error = r
        .run(context(), request(1), Arc::new(TestHost::default()))
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::MaxTurns);
    let partial = error.partial.unwrap();
    assert_eq!(partial.usage.input_tokens, 10);
    assert!(partial.final_output.is_none());
    for limits in [
        Limits {
            max_tokens: Some(12),
            max_cost: None,
        },
        Limits {
            max_tokens: None,
            max_cost: Some(1.0),
        },
    ] {
        let config = RunnerConfig {
            limits,
            cost_estimator: Some(Arc::new(UnitCost)),
            ..RunnerConfig::default()
        };
        let error = Runner::new(agent(TestModel::with(vec![Ok(answer("done"))])), config)
            .unwrap()
            .run(context(), request(2), Arc::new(TestHost::default()))
            .await
            .err()
            .unwrap();
        assert_eq!(error.error.info.category, ErrorCategory::Guardrail);
        assert_eq!(error.partial.unwrap().responses.len(), 1);
    }
}

#[tokio::test]
async fn authorization_and_argument_validation_precede_effects() {
    for mode in 0..3 {
        let mut tool = TestTool::new("write", mode == 1, false);
        if mode == 0 {
            Arc::get_mut(&mut tool).unwrap().definition.read_only = false;
        }
        let mut c = call("1", "write");
        if mode == 2 {
            if let RunItem::ToolCall { call } = &mut c {
                call.arguments = json!({"extra":true});
            }
        }
        let model = TestModel::with(vec![Ok(response(vec![c], None))]);
        let mut a = agent(model.clone());
        a.tools = vec![tool.clone()];
        let host = Arc::new(TestHost::default());
        host.approvals
            .lock()
            .unwrap()
            .push_back(ApprovalDecision::Deny);
        let error = runner(a)
            .run(context(), request(2), host)
            .await
            .err()
            .unwrap();
        assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            error.error.info.category,
            [
                ErrorCategory::PermissionDenied,
                ErrorCategory::ApprovalDenied,
                ErrorCategory::ModelBehavior
            ][mode]
        );
        assert_eq!(
            model.requests.lock().unwrap()[0].tools.len(),
            usize::from(mode != 0)
        );
    }
}

#[tokio::test]
async fn handoff_preempts_siblings_and_pairs_all_calls() {
    let source = TestModel::with(vec![Ok(response(
        vec![
            call("1", "effect"),
            call("2", "transfer"),
            call("3", "effect"),
        ],
        None,
    ))]);
    let target_model = TestModel::with(vec![Ok(answer("target"))]);
    let effect = TestTool::new("effect", false, false);
    let definition = TestTool::new("transfer", false, false).definition.clone();
    let mut target = agent(target_model.clone());
    target.name = "target".into();
    let mut a = agent(source);
    a.tools = vec![effect.clone()];
    a.handoffs = vec![Handoff {
        definition,
        target: Arc::new(target),
    }];
    let result = runner(a)
        .run(context(), request(3), Arc::new(TestHost::default()))
        .await
        .unwrap();
    assert_eq!(result.result.last_agent.as_deref(), Some("target"));
    assert_eq!(effect.calls.load(Ordering::SeqCst), 0);
    let history = &target_model.requests.lock().unwrap()[0].input;
    for id in ["1", "3"] {
        assert!(history.iter().any(|i| matches!(i, RunItem::ToolResult { call_id, output } if call_id == id && output.is_error)));
    }
    assert!(
        history
            .iter()
            .any(|i| matches!(i, RunItem::Handoff { call_id, .. } if call_id == "2"))
    );
}

#[test]
fn duplicate_names_and_invalid_schema_are_rejected() {
    let mut a = agent(TestModel::with(vec![]));
    a.tools = vec![
        TestTool::new("same", false, false),
        TestTool::new("same", false, false),
    ];
    assert_eq!(
        Runner::new(a, RunnerConfig::default())
            .err()
            .unwrap()
            .info
            .category,
        ErrorCategory::InvalidInput
    );
    let mut a = agent(TestModel::with(vec![]));
    a.output_schema = Some(schemars::json_schema!({"type":27}));
    assert!(Runner::new(a, RunnerConfig::default()).is_err());
}

#[tokio::test]
async fn structured_output_validation_preserves_baseline_result() {
    for output in ["not json", "{\"n\":-1}", "{\"n\":1}"] {
        let mut a = agent(TestModel::with(vec![Ok(answer(output))]));
        a.output_schema = Some(
            schemars::json_schema!({"type":"object", "required":["n"], "properties":{"n":{"type":"integer", "minimum":0}}, "additionalProperties":false}),
        );
        let hooks = Arc::new(Observations::default());
        let result = Runner::new(
            a,
            RunnerConfig {
                hooks: Some(hooks.clone()),
                ..RunnerConfig::default()
            },
        )
        .unwrap()
        .run(context(), request(1), Arc::new(TestHost::default()))
        .await;
        assert_eq!(
            result.unwrap().result.final_output,
            Some(serde_json::from_str(output).unwrap_or_else(|_| json!(output)))
        );
        assert_eq!(
            hooks
                .seen
                .lock()
                .unwrap()
                .iter()
                .filter(|event| matches!(event, Observation::OutputValidationFailed { .. }))
                .count(),
            usize::from(output != "{\"n\":1}")
        );
    }
}

struct Compact;
impl Compactor for Compact {
    fn compact<'a>(
        &'a self,
        _: &'a Context,
        request: CompactionRequest,
    ) -> BoxFuture<'a, Result<CompactedHistory, Error>> {
        Box::pin(async move {
            assert_eq!(request.context_tokens, 12);
            assert_eq!(request.target_tokens, 5);
            assert!(
                !request
                    .history
                    .contains(&message(Role::Developer, "dynamic"))
            );
            Ok(CompactedHistory {
                history: vec![message(Role::User, "summary")],
                context_tokens: 4,
                usage: Usage::default(),
                cost: 0.0,
            })
        })
    }
}
struct Hints;
impl TurnContext for Hints {
    fn context<'a>(
        &'a self,
        _: &'a Context,
        _: &'a RunResult,
        _: u32,
    ) -> BoxFuture<'a, Result<Vec<RunItem>, Error>> {
        Box::pin(async { Ok(vec![message(Role::Developer, "dynamic")]) })
    }
}
#[derive(Default)]
struct Observations {
    seen: Mutex<Vec<Observation>>,
}
impl RunHooks for Observations {
    fn observe<'a>(
        &'a self,
        _: &'a Context,
        event: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(event);
            Ok(())
        })
    }
}
#[tokio::test]
async fn compaction_replaces_history_and_hints_cache_prefix_are_request_only() {
    let model = TestModel::with(vec![
        Ok(response(
            vec![message(Role::Assistant, "more")],
            Some(false),
        )),
        Ok(answer("done")),
    ]);
    let hooks = Arc::new(Observations::default());
    let config = RunnerConfig {
        cache_prefix: "stable".into(),
        prompt_cache_key: Some("logical".into()),
        transient_context: vec![message(Role::Developer, "static")],
        turn_context: Some(Arc::new(Hints)),
        hooks: Some(hooks.clone()),
        compaction: Some(CompactionConfig {
            trigger_tokens: 10,
            target_tokens: 5,
            compactor: Arc::new(Compact),
        }),
        ..RunnerConfig::default()
    };
    let result = Runner::new(agent(model.clone()), config)
        .unwrap()
        .run(context(), request(3), Arc::new(TestHost::default()))
        .await
        .unwrap();
    assert_eq!(
        result.result.history,
        vec![
            message(Role::User, "summary"),
            message(Role::Assistant, "done")
        ]
    );
    assert_eq!(result.result.new_items.len(), 2);
    let requests = model.requests.lock().unwrap();
    assert_eq!(
        requests[0].settings["prompt_cache_key"],
        requests[1].settings["prompt_cache_key"]
    );
    assert_ne!(
        requests[0].settings["prompt_cache_key"],
        Value::String("logical".into())
    );
    for req in requests.iter() {
        assert!(req.instructions.starts_with("stable"));
        assert!(req.input.contains(&message(Role::Developer, "dynamic")));
    }
    assert!(hooks.seen.lock().unwrap().iter().any(|o| matches!(
        o,
        Observation::Compacted {
            before_items: 2,
            after_items: 1,
            ..
        }
    )));
}

struct Durable {
    boundaries: Mutex<Vec<Boundary>>,
    fail: Option<Boundary>,
}
impl DurableHook for Durable {
    fn checkpoint<'a>(
        &'a self,
        _: &'a Context,
        boundary: Boundary,
        snapshot: &'a RunResult,
        _: Option<&'a ToolCall>,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.boundaries.lock().unwrap().push(boundary);
            if boundary == Boundary::ToolCompleted {
                assert!(
                    snapshot
                        .history
                        .iter()
                        .any(|i| matches!(i, RunItem::ToolResult { .. }))
                );
            }
            if self.fail == Some(boundary) {
                Err(Error::new(ErrorCategory::Host, "durability failure"))
            } else {
                Ok(())
            }
        })
    }
}
#[tokio::test]
async fn durable_failure_before_effect_fails_closed_after_effect_preserves_result() {
    for boundary in [Boundary::ToolPrepared, Boundary::ToolCompleted] {
        let tool = TestTool::new("effect", false, false);
        let mut a = agent(TestModel::with(vec![Ok(response(
            vec![call("1", "effect")],
            None,
        ))]));
        a.tools = vec![tool.clone()];
        let durable = Arc::new(Durable {
            boundaries: Mutex::new(vec![]),
            fail: Some(boundary),
        });
        let config = RunnerConfig {
            durable: Some(durable.clone()),
            ..RunnerConfig::default()
        };
        let error = Runner::new(a, config)
            .unwrap()
            .run(context(), request(2), Arc::new(TestHost::default()))
            .await
            .err()
            .unwrap();
        assert_eq!(error.error.info.category, ErrorCategory::Host);
        let completed = boundary == Boundary::ToolCompleted;
        assert_eq!(tool.calls.load(Ordering::SeqCst), usize::from(completed));
        assert_eq!(
            error
                .partial
                .unwrap()
                .history
                .iter()
                .any(|i| matches!(i, RunItem::ToolResult { .. })),
            completed
        );
        assert_eq!(
            *durable.boundaries.lock().unwrap().last().unwrap(),
            boundary
        );
    }
}

#[tokio::test]
async fn raw_hooks_precede_processing_and_spills_live_across_pause_and_error() {
    let mut tool = TestTool::new("big", false, true);
    Arc::get_mut(&mut tool).unwrap().output.content = vec![Content::Text {
        text: "x".repeat(2000),
    }];
    let model = TestModel::with(vec![
        Ok(response(vec![call("1", "big")], None)),
        Err(provider_error()),
    ]);
    let mut a = agent(model);
    a.tools = vec![tool];
    let hooks = Arc::new(Observations::default());
    let mut config = RunnerConfig {
        hooks: Some(hooks.clone()),
        ..RunnerConfig::default()
    };
    config.output.max_bytes = Some(512);
    let r = Runner::new(a, config).unwrap();
    let mut req = request(3);
    req.policy.tools.access = AccessMode::WorkspaceWrite;
    let mut paused = r
        .run(context(), req, Arc::new(TestHost::default()))
        .await
        .unwrap();
    let path = paused.spills[0].path().to_path_buf();
    assert!(path.exists());
    let raw = hooks
        .seen
        .lock()
        .unwrap()
        .iter()
        .find_map(|o| {
            if let Observation::RawToolOutput { output, .. } = o {
                Some(output.clone())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(
        raw.content,
        vec![Content::Text {
            text: "x".repeat(2000)
        }]
    );
    let continuation = paused.continuation.take().unwrap();
    drop(paused);
    assert!(path.exists());
    let error = continuation.resume(None).await.err().unwrap();
    assert!(path.exists());
    assert_eq!(error.error.info.category, ErrorCategory::Provider);
    assert!(error.partial.as_ref().unwrap().history.iter().any(|i| matches!(i, RunItem::ToolResult { output, .. } if matches!(&output.content[0], Content::Text {text} if text.len() <= 512 && text.contains("UNTRUSTED")))));
    drop(error);
    assert!(!path.exists());
}

struct SlowHost;
impl Host for SlowHost {
    fn emit<'a>(&'a self, _: &'a Context, event: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            if matches!(event, RunEvent::Model { .. }) {
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
            Ok(())
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

#[tokio::test]
async fn host_backpressure_does_not_consume_stream_idle_budget() {
    let model = TestModel::streaming(vec![
        StreamStep::Event(ModelEvent::TextDelta { delta: "ok".into() }),
        StreamStep::Event(ModelEvent::Complete {
            response: answer("ok"),
        }),
    ]);
    let config = RunnerConfig {
        model_idle_timeout: Some(Duration::from_millis(5)),
        ..RunnerConfig::default()
    };
    let result = Runner::new(agent(model), config)
        .unwrap()
        .stream(context(), request(1), Arc::new(SlowHost))
        .finish()
        .await
        .unwrap();
    assert_eq!(result.result.final_output, Some(json!("ok")));
}

struct FailingHost;
impl Host for FailingHost {
    fn emit<'a>(&'a self, _: &'a Context, event: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            if matches!(
                event,
                RunEvent::Model {
                    event: ModelEvent::Complete { .. }
                }
            ) {
                Err(Error::new(ErrorCategory::Host, "sink failed"))
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
        Box::pin(async { unreachable!() })
    }
}
#[tokio::test]
async fn host_complete_event_failure_keeps_response_in_both_modes() {
    for streaming in [false, true] {
        let model = TestModel::with(vec![Ok(answer("done"))]);
        model
            .streams
            .lock()
            .unwrap()
            .push_back(vec![StreamStep::Event(ModelEvent::Complete {
                response: answer("done"),
            })]);
        let r = runner(agent(model));
        let result = if streaming {
            r.stream(context(), request(1), Arc::new(FailingHost))
                .finish()
                .await
        } else {
            r.run(context(), request(1), Arc::new(FailingHost)).await
        };
        let error = result.err().unwrap();
        assert_eq!(error.error.info.category, ErrorCategory::Host);
        let partial = error.partial.unwrap();
        assert_eq!(partial.usage.input_tokens, 10);
        assert_eq!(partial.responses.len(), 1);
        assert_eq!(
            partial.history.last(),
            Some(&message(Role::Assistant, "done"))
        );
    }
}

struct RejectRaw;
impl RunHooks for RejectRaw {
    fn observe<'a>(
        &'a self,
        _: &'a Context,
        event: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            if matches!(event, Observation::RawToolOutput { .. }) {
                Err(Error::new(ErrorCategory::Guardrail, "raw output rejected"))
            } else {
                Ok(())
            }
        })
    }
}
#[tokio::test]
async fn raw_hook_failure_retains_completion_but_withholds_output() {
    let tool = TestTool::new("secret", false, false);
    let mut a = agent(TestModel::with(vec![Ok(response(
        vec![call("1", "secret")],
        None,
    ))]));
    a.tools = vec![tool.clone()];
    let config = RunnerConfig {
        hooks: Some(Arc::new(RejectRaw)),
        ..RunnerConfig::default()
    };
    let error = Runner::new(a, config)
        .unwrap()
        .run(context(), request(2), Arc::new(TestHost::default()))
        .await
        .err()
        .unwrap();
    assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
    assert_eq!(error.error.info.category, ErrorCategory::Guardrail);
    let partial = error.partial.unwrap();
    let Some(RunItem::ToolResult { output, .. }) = partial.history.last() else {
        panic!("missing completed effect")
    };
    assert!(output.is_error);
    assert_ne!(output.content, tool.output.content);
}

struct BilledCompaction;
impl Compactor for BilledCompaction {
    fn compact<'a>(
        &'a self,
        _: &'a Context,
        _: CompactionRequest,
    ) -> BoxFuture<'a, Result<CompactedHistory, Error>> {
        Box::pin(async {
            Ok(CompactedHistory {
                history: vec![message(Role::User, "summary")],
                context_tokens: 4,
                usage: Usage {
                    input_tokens: 20,
                    output_tokens: 5,
                    ..Usage::default()
                },
                cost: 2.0,
            })
        })
    }
}
#[tokio::test]
async fn provider_compaction_is_charged_before_the_next_model_turn() {
    for limits in [
        Limits {
            max_tokens: Some(30),
            max_cost: None,
        },
        Limits {
            max_tokens: None,
            max_cost: Some(2.5),
        },
    ] {
        let model = TestModel::with(vec![Ok(response(
            vec![message(Role::Assistant, "more")],
            Some(false),
        ))]);
        let config = RunnerConfig {
            limits,
            cost_estimator: Some(Arc::new(UnitCost)),
            compaction: Some(CompactionConfig {
                trigger_tokens: 10,
                target_tokens: 5,
                compactor: Arc::new(BilledCompaction),
            }),
            ..RunnerConfig::default()
        };
        let error = Runner::new(agent(model.clone()), config)
            .unwrap()
            .run(context(), request(3), Arc::new(TestHost::default()))
            .await
            .err()
            .unwrap();
        assert_eq!(error.error.info.category, ErrorCategory::Guardrail);
        assert_eq!(model.completes.load(Ordering::SeqCst), 1);
        let partial = error.partial.unwrap();
        assert_eq!(partial.usage.input_tokens, 30);
        assert_eq!(partial.usage.output_tokens, 7);
        assert_eq!(partial.history, vec![message(Role::User, "summary")]);
    }
}

#[tokio::test]
async fn cancellation_interrupts_backoff_without_another_attempt() {
    let model = TestModel::with(vec![Err(provider_error())]);
    let cancellation = CancellationToken::new();
    let signal = cancellation.clone();
    let mut ctx = context();
    ctx.cancellation = Arc::new(cancellation);
    let config = RunnerConfig {
        retry: RetryPolicy {
            max_retries: 5,
            initial_delay: Duration::from_secs(60),
            max_delay: Duration::from_secs(60),
            ..RetryPolicy::default()
        },
        ..RunnerConfig::default()
    };
    let r = Runner::new(agent(model.clone()), config).unwrap();
    let (result, ()) = tokio::join!(
        r.run(ctx, request(1), Arc::new(TestHost::default())),
        async {
            tokio::time::sleep(Duration::from_millis(5)).await;
            signal.cancel();
        }
    );
    assert_eq!(
        result.err().unwrap().error.info.category,
        ErrorCategory::Cancelled
    );
    assert_eq!(model.completes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn conversation_keeps_failed_spill_history_alive_after_error_is_dropped() {
    let mut tool = TestTool::new("big", false, false);
    Arc::get_mut(&mut tool).unwrap().output.content = vec![Content::Text {
        text: "x".repeat(2000),
    }];
    let mut a = agent(TestModel::with(vec![
        Ok(response(vec![call("1", "big")], None)),
        Err(provider_error()),
    ]));
    a.tools = vec![tool];
    let mut config = RunnerConfig::default();
    config.output.max_bytes = Some(512);
    let mut conversation = Conversation::default();
    let mut policy = policy(3);
    policy.tools.access = AccessMode::WorkspaceWrite;
    let error = conversation
        .run(
            &Runner::new(a, config).unwrap(),
            context(),
            vec![message(Role::User, "go")],
            policy,
            Arc::new(TestHost::default()),
        )
        .await
        .err()
        .unwrap();
    let serialized = serde_json::to_string(&conversation.history).unwrap();
    assert!(serialized.contains("full output saved to"));
    drop(error);
    let Some(RunItem::ToolResult { output, .. }) = conversation.history.last() else {
        panic!("no result")
    };
    let Content::Text { text } = &output.content[0] else {
        panic!("no text")
    };
    let path = text
        .split("[full output saved to ")
        .nth(1)
        .unwrap()
        .split(']')
        .next()
        .unwrap();
    let path = std::path::PathBuf::from(path);
    assert!(path.exists());
    drop(conversation);
    assert!(!path.exists());
}

#[tokio::test]
async fn sticky_fallback_survives_approval_resume_and_reprobes_after_three_successes() {
    let primary = TestModel::with(vec![Err(provider_error()), Ok(answer("primary recovered"))]);
    let backup = TestModel::with(vec![
        Ok(response(vec![call("1", "approved")], None)),
        Ok(response(vec![], Some(false))),
        Ok(response(vec![], Some(false))),
    ]);
    let tool = TestTool::new("approved", true, false);
    let mut a = agent(primary.clone());
    a.fallbacks = vec![ModelBinding::complete("backup", backup.clone())];
    a.tools = vec![tool.clone()];
    let host = Arc::new(TestHost::default());
    host.approvals
        .lock()
        .unwrap()
        .push_back(ApprovalDecision::Defer);
    let paused = runner(a).run(context(), request(4), host).await.unwrap();
    assert_eq!(primary.completes.load(Ordering::SeqCst), 1);
    let done = paused
        .continuation
        .unwrap()
        .resume(Some(ApprovalDecision::Approve))
        .await
        .unwrap();
    assert_eq!(done.result.final_output, Some(json!("primary recovered")));
    assert_eq!(primary.completes.load(Ordering::SeqCst), 2);
    assert_eq!(backup.completes.load(Ordering::SeqCst), 3);
    assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
}

struct BatchTool {
    definition: ToolDefinition,
    barrier: Option<Arc<tokio::sync::Barrier>>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    pending: bool,
}
impl Tool for BatchTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            struct Guard<'a>(&'a AtomicUsize, &'a AtomicUsize);
            impl Drop for Guard<'_> {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::SeqCst);
                    self.1.fetch_add(1, Ordering::SeqCst);
                }
            }
            let n = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            let _guard = Guard(&self.active, &self.drops);
            self.peak.fetch_max(n, Ordering::SeqCst);
            if let Some(barrier) = &self.barrier {
                barrier.wait().await;
            }
            if self.pending {
                std::future::pending::<()>().await;
            }
            tokio::task::yield_now().await;
            Ok(ToolOutput {
                content: vec![Content::Text {
                    text: self.definition.name.clone(),
                }],
                is_error: false,
                should_pause: false,
            })
        })
    }
}

#[tokio::test]
async fn tool_batches_fan_out_reads_exclude_mutations_and_fold_in_call_order() {
    for read_only in [true, false] {
        for streaming in [false, true] {
            let active = Arc::new(AtomicUsize::new(0));
            let peak = Arc::new(AtomicUsize::new(0));
            let drops = Arc::new(AtomicUsize::new(0));
            let barrier = read_only.then(|| Arc::new(tokio::sync::Barrier::new(2)));
            let calls = response(vec![call("1", "one"), call("2", "two")], None);
            let model = if streaming {
                TestModel::streaming(vec![StreamStep::Event(ModelEvent::Complete {
                    response: calls,
                })])
            } else {
                TestModel::with(vec![Ok(calls)])
            };
            let mut a = agent(model);
            a.tools = ["one", "two"]
                .into_iter()
                .map(|name| {
                    Arc::new(BatchTool {
                        definition: ToolDefinition {
                            name: name.into(),
                            description: String::new(),
                            input_schema: schemars::json_schema!({"type":"object"}),
                            read_only,
                            requires_approval: false,
                        },
                        barrier: barrier.clone(),
                        active: active.clone(),
                        peak: peak.clone(),
                        drops: drops.clone(),
                        pending: false,
                    }) as Arc<dyn Tool>
                })
                .collect();
            let mut req = request(1);
            req.policy.tool_use = ToolUseBehavior::StopAfterTool;
            req.policy.tools.access = AccessMode::FullAccess;
            let runner = runner(a);
            let execution = async {
                if streaming {
                    runner
                        .stream(context(), req, Arc::new(TestHost::default()))
                        .finish()
                        .await
                } else {
                    runner
                        .run(context(), req, Arc::new(TestHost::default()))
                        .await
                }
            };
            let done = tokio::time::timeout(Duration::from_secs(1), execution)
                .await
                .expect("sequential reads deadlocked at barrier")
                .unwrap();
            assert_eq!(peak.load(Ordering::SeqCst), if read_only { 2 } else { 1 });
            assert_eq!(drops.load(Ordering::SeqCst), 2);
            assert_eq!(active.load(Ordering::SeqCst), 0);
            let ids: Vec<_> = done
                .result
                .history
                .iter()
                .filter_map(|item| match item {
                    RunItem::ToolResult { call_id, .. } => Some(call_id.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(ids, ["1", "2"]);
        }
    }
}

#[tokio::test]
async fn dropping_stream_drops_all_inflight_batch_tools() {
    let active = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let model = TestModel::streaming(vec![StreamStep::Event(ModelEvent::Complete {
        response: response(vec![call("1", "one"), call("2", "two")], None),
    })]);
    let mut a = agent(model);
    a.tools = ["one", "two"]
        .into_iter()
        .map(|name| {
            Arc::new(BatchTool {
                definition: ToolDefinition {
                    name: name.into(),
                    description: String::new(),
                    input_schema: schemars::json_schema!({"type":"object"}),
                    read_only: true,
                    requires_approval: false,
                },
                barrier: None,
                active: active.clone(),
                peak: Arc::new(AtomicUsize::new(0)),
                drops: drops.clone(),
                pending: true,
            }) as Arc<dyn Tool>
        })
        .collect();
    let mut stream = runner(a).stream(context(), request(2), Arc::new(TestHost::default()));
    while tokio::time::timeout(Duration::from_millis(10), stream.next())
        .await
        .is_ok()
    {}
    assert_eq!(active.load(Ordering::SeqCst), 2);
    drop(stream);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn fallback_state_is_per_agent_identity_not_display_name() {
    let primary = TestModel::with(vec![Err(provider_error())]);
    let backup = TestModel::with(vec![Ok(response(vec![call("1", "transfer")], None))]);
    let target_primary = TestModel::with(vec![Ok(answer("target primary"))]);
    let target = agent(target_primary.clone());
    let mut source = agent(primary);
    assert_eq!(source.name, target.name);
    source.fallbacks = vec![ModelBinding::complete("backup", backup)];
    source.handoffs = vec![Handoff {
        definition: TestTool::new("transfer", false, false).definition.clone(),
        target: Arc::new(target),
    }];
    let result = runner(source)
        .run(context(), request(2), Arc::new(TestHost::default()))
        .await
        .unwrap();
    assert_eq!(result.result.final_output, Some(json!("target primary")));
    assert_eq!(target_primary.completes.load(Ordering::SeqCst), 1);
}
