use adk_core::*;
use adk_runtime::{progress::*, *};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

fn cx() -> Context {
    Context {
        run_id: "progress".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn request() -> RunRequest {
    RunRequest {
        input: vec![],
        input_provenance: vec![],
        policy: RunPolicy::default(),
    }
}
fn message(text: &str) -> RunItem {
    RunItem::Message {
        message: Message {
            role: Role::Assistant,
            content: vec![Content::Text { text: text.into() }],
        },
    }
}
fn response(items: Vec<RunItem>) -> ModelResponse {
    ModelResponse {
        items,
        usage: Usage {
            requests: 1,
            input_tokens: 13,
            output_tokens: 7,
            cache_read_tokens: 5,
            cache_creation_tokens: 3,
            context_tokens: Some(100),
        },
        end_turn: None,
        response_id: None,
        metadata: Default::default(),
        raw: None,
        snapshot_raw: None,
        snapshot_projection: None,
    }
}
struct HostImpl;
impl Host for HostImpl {
    fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { Ok(ApprovalDecision::Defer) })
    }
}
struct Fake {
    answers: Mutex<VecDeque<Result<ModelResponse, Error>>>,
    fail_stream: bool,
}
impl Fake {
    fn new(answers: Vec<ModelResponse>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into_iter().map(Ok).collect()),
            fail_stream: false,
        })
    }
}
impl Model for Fake {
    fn provider(&self) -> &str {
        "provider"
    }
    fn retry_advice(&self, error: &Error) -> Option<ModelRetryAdvice> {
        (error.info.message == "overloaded").then(|| ModelRetryAdvice {
            should_retry: true,
            retry_after: Duration::ZERO,
            reason: "overloaded".into(),
        })
    }
    fn info(&self, _: &str) -> ModelInfo {
        ModelInfo {
            provider: "provider".into(),
            model: "canonical".into(),
            input_tokens_include_cache: None,
        }
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async { self.answers.lock().unwrap().pop_front().unwrap() })
    }
}
struct Events(VecDeque<Result<ModelEvent, Error>>);
impl ModelStream for Events {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async { self.0.pop_front().transpose() })
    }
}
impl StreamingModel for Fake {
    fn stream<'a>(
        &'a self,
        cx: &'a Context,
        req: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            let answer = self.complete(cx, req).await?;
            let mut events = VecDeque::from([Ok(ModelEvent::Complete { response: answer })]);
            if self.fail_stream {
                events.push_back(Err(Error::new(ErrorCategory::Provider, "after complete")));
            }
            Ok(Box::new(Events(events)) as Box<dyn ModelStream>)
        })
    }
}
type UsageRecord = (UsageOrigin, String, String, String, String, Usage);

#[derive(Default)]
struct Seen {
    events: Mutex<Vec<String>>,
    usage: Mutex<Vec<UsageRecord>>,
}
impl RuntimeProgress for Seen {
    fn observe(&self, _: &Context, _: ProgressFrame<'_>, event: ProgressObservation<'_>) {
        let name = match event {
            ProgressObservation::Lifecycle(Observation::ModelAccepted { .. }) => "accepted",
            ProgressObservation::Lifecycle(Observation::Usage { .. }) => "cumulative",
            ProgressObservation::Retry {
                attempt,
                max_retries,
                ..
            } => {
                assert_eq!(attempt, 1);
                assert_eq!(max_retries, 1);
                "retry"
            }
            _ => return,
        };
        self.events.lock().unwrap().push(name.into());
    }
    fn usage(&self, _: &Context, frame: ProgressFrame<'_>, call: ProgressUsage<'_>) {
        self.events
            .lock()
            .unwrap()
            .push(format!("{:?}", call.origin));
        self.usage.lock().unwrap().push((
            call.origin,
            frame.agent_model.into(),
            call.request_model.into(),
            call.provider.into(),
            call.canonical_model.into(),
            call.usage.clone(),
        ));
    }
}
#[tokio::test]
async fn accepted_and_retained_failure_are_distinct_without_generation_observer() {
    for failed in [false, true] {
        let seen = Arc::new(Seen::default());
        let model = Arc::new(Fake {
            answers: Mutex::new(VecDeque::from([Ok(response(vec![message("done")]))])),
            fail_stream: failed,
        });
        let runner = Runner::new(
            AgentConfig::new("agent", ModelBinding::streaming("configured", model)),
            RunnerConfig {
                progress: Some(seen.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let outcome = runner
            .stream(cx(), request(), Arc::new(HostImpl))
            .finish()
            .await;
        assert_eq!(outcome.is_err(), failed);
        let usage = seen.usage.lock().unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(
            usage[0].0,
            if failed {
                UsageOrigin::FailedStreamRetained
            } else {
                UsageOrigin::Accepted
            }
        );
        assert_eq!(
            (&usage[0].1, &usage[0].2, &usage[0].3, &usage[0].4),
            (
                &"configured".into(),
                &"configured".into(),
                &"provider".into(),
                &"canonical".into()
            )
        );
        assert_eq!(usage[0].5.input_tokens, 13);
        let events = seen.events.lock().unwrap();
        if failed {
            assert!(!events.iter().any(|event| event == "accepted"));
        } else {
            assert_eq!(*events, ["Accepted", "accepted", "cumulative"]);
        }
    }
}
#[tokio::test]
async fn fallback_preserves_primary_label_and_retry_details() {
    let seen = Arc::new(Seen::default());
    let bad = Arc::new(Fake {
        answers: Mutex::new(VecDeque::from([Err(Error::new(
            ErrorCategory::Provider,
            "overloaded",
        ))])),
        fail_stream: false,
    });
    let mut agent = AgentConfig::new("agent", ModelBinding::complete("primary", bad));
    agent.fallbacks.push(ModelBinding::complete(
        "fallback",
        Fake::new(vec![response(vec![message("done")])]),
    ));
    Runner::new(
        agent,
        RunnerConfig {
            progress: Some(seen.clone()),
            retry: RetryPolicy {
                max_retries: 1,
                initial_delay: Duration::ZERO,
                max_delay: Duration::ZERO,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap()
    .run(cx(), request(), Arc::new(HostImpl))
    .await
    .unwrap();
    let calls = seen.usage.lock().unwrap();
    assert_eq!(
        (&calls[0].1, &calls[0].2),
        (&"primary".into(), &"fallback".into())
    );
}
struct Compact;
impl Compactor for Compact {
    fn compact<'a>(
        &'a self,
        _: &'a Context,
        req: CompactionRequest,
    ) -> BoxFuture<'a, Result<CompactedHistory, Error>> {
        Box::pin(async move {
            Ok(CompactedHistory {
                context_tokens: 1,
                history: req.history,
                history_provenance: req.history_provenance,
                usage: Usage {
                    input_tokens: 2,
                    output_tokens: 1,
                    context_tokens: Some(1),
                    ..Default::default()
                },
                cost: 7.0,
                cost_known: true,
            })
        })
    }
}
#[tokio::test]
async fn provider_compaction_is_separate_and_cumulative_usage_unchanged() {
    let seen = Arc::new(Seen::default());
    let call = RunItem::ToolCall {
        call: ToolCall {
            id: "tool".into(),
            name: "unknown".into(),
            arguments: serde_json::json!({}),
            raw_arguments: None,
        },
    };
    let model = Fake::new(vec![response(vec![call]), response(vec![message("done")])]);
    let runner = Runner::new(
        AgentConfig::new("agent", ModelBinding::complete("original", model)),
        RunnerConfig {
            progress: Some(seen.clone()),
            compaction: Some(CompactionConfig {
                trigger_tokens: 50,
                target_tokens: 20,
                compactor: Arc::new(Compact),
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let outcome = runner
        .run(cx(), request(), Arc::new(HostImpl))
        .await
        .unwrap();
    let origins: Vec<_> = seen
        .usage
        .lock()
        .unwrap()
        .iter()
        .map(|call| call.0)
        .collect();
    assert_eq!(
        origins,
        [
            UsageOrigin::Accepted,
            UsageOrigin::ProviderCompaction,
            UsageOrigin::Accepted
        ]
    );
    assert_eq!(outcome.result.usage.input_tokens, 28);
}
#[tokio::test]
async fn summary_compaction_reports_actual_request_model() {
    use adk_runtime::compaction::LocalCompactionPolicy;
    let seen = Arc::new(Seen::default());
    let model = Fake::new(vec![
        response(vec![message("summary")]),
        response(vec![message("done")]),
    ]);
    let runner = Runner::new(
        AgentConfig::new("agent", ModelBinding::complete("original", model)),
        RunnerConfig {
            progress: Some(seen.clone()),
            local_compaction: LocalCompactionPolicy {
                trigger_tokens: 30_000,
                target_tokens: 20_000,
                preserve_initial_user_messages: 1,
                preserve_recent_items: 1,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    let mut req = request();
    req.input = vec![
        RunItem::Message {
            message: Message {
                role: Role::User,
                content: vec![Content::Text {
                    text: "task".into(),
                }],
            },
        },
        message(&"removed evidence ".repeat(4_000)),
        RunItem::Message {
            message: Message {
                role: Role::User,
                content: vec![Content::Text {
                    text: "latest".into(),
                }],
            },
        },
    ];
    runner.run(cx(), req, Arc::new(HostImpl)).await.unwrap();
    let usage = seen.usage.lock().unwrap();
    assert_eq!(
        usage.iter().map(|call| call.0).collect::<Vec<_>>(),
        [UsageOrigin::SummaryCompaction, UsageOrigin::Accepted]
    );
    assert_eq!(usage[0].2, "original");
}

#[tokio::test]
async fn retries_report_attempt_and_delay_without_usage() {
    let seen = Arc::new(Seen::default());
    let model = Arc::new(Fake {
        answers: Mutex::new(VecDeque::from([
            Err(Error::new(ErrorCategory::Provider, "temporary")),
            Ok(response(vec![message("done")])),
        ])),
        fail_stream: false,
    });
    Runner::new(
        AgentConfig::new("agent", ModelBinding::complete("model", model)),
        RunnerConfig {
            progress: Some(seen.clone()),
            retry: RetryPolicy {
                max_retries: 1,
                initial_delay: Duration::ZERO,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap()
    .run(cx(), request(), Arc::new(HostImpl))
    .await
    .unwrap();
    assert_eq!(
        *seen.events.lock().unwrap(),
        ["retry", "Accepted", "accepted", "cumulative"]
    );
}
struct Signal(Arc<tokio::sync::Notify>);
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
struct Interrupting {
    wake: Arc<tokio::sync::Notify>,
    starts: std::sync::atomic::AtomicUsize,
    completed: bool,
}
impl Model for Interrupting {
    fn provider(&self) -> &str {
        "provider"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        panic!("stream required")
    }
}
struct InterruptedEvents {
    step: u32,
    completed: bool,
    wake: Option<Arc<tokio::sync::Notify>>,
}
impl ModelStream for InterruptedEvents {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async move {
            self.step += 1;
            if self.step == 1 && self.completed {
                return Ok(Some(ModelEvent::Complete {
                    response: response(vec![message("complete")]),
                }));
            }
            if let Some(wake) = self.wake.take() {
                wake.notify_one();
                std::future::pending::<()>().await;
            }
            Ok(None)
        })
    }
}
impl StreamingModel for Interrupting {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            let first = self
                .starts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                == 0;
            Ok(Box::new(InterruptedEvents {
                step: 0,
                completed: !first || self.completed,
                wake: first.then(|| self.wake.clone()),
            }) as Box<dyn ModelStream>)
        })
    }
}
#[tokio::test]
async fn steering_only_reports_genuinely_completed_superseded_usage() {
    for completed in [false, true] {
        let seen = Arc::new(Seen::default());
        let wake = Arc::new(tokio::sync::Notify::new());
        let model = Arc::new(Interrupting {
            wake: wake.clone(),
            starts: Default::default(),
            completed,
        });
        let result = Runner::new(
            AgentConfig::new("agent", ModelBinding::streaming("primary", model)),
            RunnerConfig {
                progress: Some(seen.clone()),
                blocking_reasoning: true,
                immediate_input_signal: Some(Arc::new(Signal(wake))),
                immediate_input_poller: Some(Arc::new(Poller)),
                ..Default::default()
            },
        )
        .unwrap()
        .run(cx(), request(), Arc::new(HostImpl))
        .await
        .unwrap();
        let origins: Vec<_> = seen
            .usage
            .lock()
            .unwrap()
            .iter()
            .map(|usage| usage.0)
            .collect();
        assert_eq!(
            origins,
            if completed {
                vec![UsageOrigin::SteeringSuperseded, UsageOrigin::Accepted]
            } else {
                vec![UsageOrigin::Accepted]
            }
        );
        assert_eq!(
            result.result.usage.input_tokens, 13,
            "native cumulative accounting is unchanged"
        );
    }
}
struct ApprovalTool(ToolDefinition);
impl Tool for ApprovalTool {
    fn definition(&self) -> &ToolDefinition {
        &self.0
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async {
            Ok(ToolOutput {
                content: vec![],
                is_error: false,
                should_pause: false,
            })
        })
    }
}
#[tokio::test]
async fn approval_resume_does_not_republish_accepted_usage() {
    let seen = Arc::new(Seen::default());
    let tool = ToolCall {
        id: "approval".into(),
        name: "approved".into(),
        arguments: serde_json::json!({}),
        raw_arguments: None,
    };
    let model = Fake::new(vec![
        response(vec![RunItem::ToolCall { call: tool }]),
        response(vec![message("done")]),
    ]);
    let mut agent = AgentConfig::new("agent", ModelBinding::complete("primary", model));
    agent.tools.push(Arc::new(ApprovalTool(ToolDefinition {
        name: "approved".into(),
        description: "approval".into(),
        input_schema: schemars::schema_for!(serde_json::Value),
        read_only: true,
        requires_approval: true,
    })));
    let outcome = Runner::new(
        agent,
        RunnerConfig {
            progress: Some(seen.clone()),
            ..Default::default()
        },
    )
    .unwrap()
    .run(cx(), request(), Arc::new(HostImpl))
    .await
    .unwrap();
    assert_eq!(seen.usage.lock().unwrap().len(), 1);
    let outcome = outcome
        .continuation
        .unwrap()
        .resume(Some(ApprovalDecision::Approve))
        .await
        .unwrap();
    assert!(outcome.continuation.is_none());
    assert_eq!(seen.usage.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn restored_descendants_run_before_ancestors_without_losing_progress() {
    use adk_runtime::{subagent::*, subagent_tools::*};
    struct Seed(tokio::sync::Notify);
    impl ChildExecutor for Seed {
        fn execute<'a>(
            &'a self,
            invocation: ChildInvocation,
            control: ChildControl,
        ) -> BoxFuture<'a, Result<ChildOutcome, Error>> {
            Box::pin(async move {
                control
                    .charge(BudgetUsage {
                        tokens: 100,
                        ..Default::default()
                    })
                    .await?;
                let children: &[&str] = match invocation.task_id.as_str() {
                    "parent" => &["middle"],
                    "middle" => &["leaf-a", "leaf-b"],
                    _ => panic!("leaf must remain queued"),
                };
                for id in children {
                    let mut submission = Submission::new("worker", "work");
                    submission.id = (*id).into();
                    control.delegation_handle().submit(submission).await?;
                }
                if invocation.task_id == "middle" {
                    self.0.notify_one();
                }
                std::future::pending().await
            })
        }
    }
    #[derive(Default)]
    struct Events {
        restored: Mutex<Vec<Vec<String>>>,
        started: Mutex<Vec<Vec<String>>>,
        finished: Mutex<Vec<Vec<String>>>,
        usage: Mutex<Vec<(Vec<String>, u64)>>,
    }
    struct Observer {
        path: Vec<String>,
        events: Arc<Events>,
        restore: bool,
    }
    impl Observer {
        fn child(&self, child: ProgressChild<'_>) -> Self {
            assert_eq!(child.parent_task_id, self.path.last().map(String::as_str));
            let mut path = self.path.clone();
            path.push(child.task_id.into());
            Self {
                path,
                events: self.events.clone(),
                restore: self.restore,
            }
        }
    }
    impl RuntimeProgress for Observer {
        fn restore_child(&self, child: ProgressChild<'_>) -> Option<Arc<dyn RuntimeProgress>> {
            if !self.restore {
                return None;
            }
            assert!(child.agent_model.is_empty());
            let child = self.child(child);
            self.events
                .restored
                .lock()
                .unwrap()
                .push(child.path.clone());
            Some(Arc::new(child))
        }
        fn fork_child(&self, child: ProgressChild<'_>) -> Option<Arc<dyn RuntimeProgress>> {
            let child = self.child(child);
            self.events.started.lock().unwrap().push(child.path.clone());
            Some(Arc::new(child))
        }
        fn child_finished(&self, child: ProgressChild<'_>, _: ProgressChildEnd<'_>) {
            self.events
                .finished
                .lock()
                .unwrap()
                .push(self.child(child).path);
        }
        fn usage(&self, _: &Context, _: ProgressFrame<'_>, usage: ProgressUsage<'_>) {
            self.events
                .usage
                .lock()
                .unwrap()
                .push((self.path.clone(), usage.usage.input_tokens));
        }
    }
    let config = SchedulerConfig {
        max_concurrency: 2,
        agents: [("worker".into(), SecurityBaseline::default())]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let seed = Arc::new(Seed(tokio::sync::Notify::new()));
    let owner = Scheduler::new(cx(), config.clone(), seed.clone(), None).unwrap();
    let mut submission = Submission::new("worker", "work");
    submission.id = "parent".into();
    owner.handle().submit(submission).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), seed.0.notified())
        .await
        .unwrap();
    let saved = owner.handle().checkpoint(&cx()).await.unwrap();
    assert_eq!(saved["usage"]["tokens"], 200);
    owner.shutdown().await.unwrap();

    for restore in [true, false] {
        let events = Arc::new(Events::default());
        let runner = Runner::new(
            AgentConfig::new(
                "worker",
                ModelBinding::complete(
                    "model",
                    Fake::new(vec![
                        response(vec![message("a")]),
                        response(vec![message("b")]),
                    ]),
                ),
            ),
            RunnerConfig::default(),
        )
        .unwrap();
        let executor = RunnerChildExecutor::new(
            [("worker".into(), runner)].into_iter().collect(),
            Arc::new(HostImpl),
        )
        .with_progress(Arc::new(ChildProgressRegistry::new(Arc::new(Observer {
            path: vec![],
            events: events.clone(),
            restore,
        }))));
        let owner = Scheduler::new(cx(), config.clone(), Arc::new(executor), None).unwrap();
        let handle = owner.handle();
        handle.restore(&cx(), saved.clone()).await.unwrap();
        for id in ["leaf-a", "leaf-b"] {
            handle.resume_queued(id).await.unwrap();
        }
        let tasks = handle
            .wait(
                &["leaf-a".into(), "leaf-b".into()],
                WaitMode::All,
                Some(Duration::from_secs(2)),
            )
            .await
            .unwrap();
        assert!(
            tasks
                .iter()
                .all(|task| task.status == TaskStatus::Completed)
        );
        assert!(tasks.iter().all(|task| task.usage.tokens == 20));
        for id in ["parent", "middle"] {
            assert_eq!(
                handle.status(id, Detail::Summary).unwrap().status,
                TaskStatus::Reconciling
            );
        }
        let paths = vec![
            vec!["parent", "middle", "leaf-a"],
            vec!["parent", "middle", "leaf-b"],
        ];
        if restore {
            assert_eq!(
                *events.restored.lock().unwrap(),
                vec![vec!["parent"], vec!["parent", "middle"]]
            );
            let mut started = events.started.lock().unwrap().clone();
            started.sort();
            assert_eq!(started, paths);
            let mut finished = events.finished.lock().unwrap().clone();
            finished.sort();
            assert_eq!(finished, paths);
            let mut usage = events.usage.lock().unwrap().clone();
            usage.sort();
            assert_eq!(
                usage,
                paths
                    .into_iter()
                    .map(|path| (path.into_iter().map(String::from).collect(), 13))
                    .collect::<Vec<_>>()
            );
        } else {
            assert!(
                events.started.lock().unwrap().is_empty(),
                "missing ancestors must never fall back to root"
            );
            assert!(events.usage.lock().unwrap().is_empty());
            assert!(events.finished.lock().unwrap().is_empty());
        }
        owner.shutdown().await.unwrap();
        assert_eq!(
            events.finished.lock().unwrap().len(),
            if restore { 2 } else { 0 }
        );
    }
}
