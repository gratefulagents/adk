#![cfg(feature = "observability")]
use adk::progress::{ProgressTracker, runtime::*};
use adk_core::*;
use adk_runtime::{progress::*, *};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
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
struct Cost;
struct GatedModel {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    first: std::sync::atomic::AtomicBool,
}
impl GatedModel {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            first: std::sync::atomic::AtomicBool::new(true),
        })
    }
}
impl Model for GatedModel {
    fn provider(&self) -> &str {
        "provider"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            if self.first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            Ok(response(vec![message("old generation")]))
        })
    }
}
impl StreamingModel for GatedModel {
    fn stream<'a>(
        &'a self,
        cx: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            let response = self.complete(cx, request).await?;
            Ok(Box::new(Events(VecDeque::from([Ok(ModelEvent::Complete {
                response,
            })]))) as Box<dyn ModelStream>)
        })
    }
}

#[cfg(feature = "builder")]
#[tokio::test]
async fn owned_refresh_rebinds_new_roots_and_preserves_accepted_generation() {
    use adk::{builder::*, providers::factory::Kind};
    use adk_runtime::subagent::*;
    for enabled in [true, false] {
        let old_tracker = ProgressTracker::new();
        let new_tracker = ProgressTracker::new();
        let old = GatedModel::new();
        let build = |model: Arc<dyn StreamingModel>, progress| {
            Builder::new(Config {
                model: "openai/base".into(),
                roles: vec![RoleSpec {
                    name: "worker".into(),
                    instructions: "work".into(),
                    tool_access: "read_only".into(),
                    ..Default::default()
                }],
                features: Some(Features {
                    subagents: SubagentFeatures {
                        task: true,
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            })
            .model("openai", Kind::OpenAi, model)
            .unwrap()
            .runner_config(RunnerConfig {
                progress,
                ..Default::default()
            })
            .subagent_host(Arc::new(HostImpl))
        };
        let owner = build(
            old.clone(),
            Some(Arc::new(RuntimeProgressAdapter::new(
                old_tracker.clone(),
                ProgressOptions::default(),
            ))),
        )
        .build_session(&cx())
        .await
        .unwrap();
        let handle = owner.handle();
        let scheduler = &handle.subagents().unwrap().scheduler;
        let first = scheduler
            .submit(Submission::new("worker", "old"))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), old.entered.notified())
            .await
            .unwrap();
        let mut queued = Submission::new("worker", "accepted before refresh");
        queued.depends_on.push(first.clone());
        let queued = scheduler.submit(queued).await.unwrap();
        let mut owner = build(
            Fake::new(vec![response(vec![message("new generation")])]),
            enabled.then(|| {
                Arc::new(RuntimeProgressAdapter::new(
                    new_tracker.clone(),
                    ProgressOptions::default(),
                )) as Arc<dyn RuntimeProgress>
            }),
        )
        .owned_session(owner)
        .build_session(&cx())
        .await
        .unwrap();
        let fresh = scheduler
            .submit(Submission::new("worker", "new"))
            .await
            .unwrap();
        let tasks = scheduler
            .wait(
                &[fresh],
                WaitMode::All,
                Some(std::time::Duration::from_secs(2)),
            )
            .await
            .unwrap();
        assert_eq!(tasks[0].status, TaskStatus::Completed);
        assert_eq!(tasks[0].result, "new generation");
        assert_eq!(old_tracker.snapshot().input_tokens, 0);
        assert_eq!(old_tracker.snapshot().agent_count, 1);
        assert_eq!(
            new_tracker.snapshot().input_tokens,
            if enabled { 13 } else { 0 }
        );
        assert_eq!(new_tracker.snapshot().agent_count, i32::from(enabled));
        old.release.notify_one();
        let tasks = scheduler
            .wait(
                &[first, queued],
                WaitMode::All,
                Some(std::time::Duration::from_secs(2)),
            )
            .await
            .unwrap();
        assert!(tasks.iter().all(|task| task.status == TaskStatus::Completed && task.result == "old generation"));
        assert_eq!(old_tracker.snapshot().input_tokens, 26);
        assert_eq!(old_tracker.snapshot().agent_count, 2);
        assert_eq!(
            new_tracker.snapshot().input_tokens,
            if enabled { 13 } else { 0 }
        );
        owner.close().await.unwrap();
    }
}

#[tokio::test]
async fn refreshed_registry_uses_actual_parent_even_when_new_root_is_disabled() {
    use adk_runtime::{subagent::*, subagent_tools::ChildProgressRegistry};
    struct CaptureParent {
        executor: RunnerChildExecutor,
        handle: Arc<Mutex<Option<SchedulerHandle>>>,
    }
    impl ChildExecutor for CaptureParent {
        fn execute<'a>(
            &'a self,
            invocation: ChildInvocation,
            control: ChildControl,
        ) -> BoxFuture<'a, Result<ChildOutcome, Error>> {
            *self.handle.lock().unwrap() = Some(control.delegation_handle());
            self.executor.execute(invocation, control)
        }
    }
    for (old_enabled, new_enabled) in [(true, true), (true, false), (false, true)] {
        let old_tracker = ProgressTracker::new();
        let new_tracker = ProgressTracker::new();
        let old = GatedModel::new();
        let registry = Arc::new(
            ChildProgressRegistry::default().rebind(old_enabled.then(|| {
                Arc::new(RuntimeProgressAdapter::new(
                    old_tracker.clone(),
                    ProgressOptions::default(),
                )) as Arc<dyn RuntimeProgress>
            })),
        );
        let executor = |model: Arc<dyn Model>, registry| {
            RunnerChildExecutor::new(
                [(
                    "worker".into(),
                    Runner::new(
                        AgentConfig::new("worker", ModelBinding::complete("model", model)),
                        RunnerConfig::default(),
                    )
                    .unwrap(),
                )]
                .into_iter()
                .collect(),
                Arc::new(HostImpl),
            )
            .with_progress(registry)
        };
        let config = SchedulerConfig {
            agents: [("worker".into(), SecurityBaseline::default())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let parent_handle = Arc::new(Mutex::new(None));
        let mut owner = Scheduler::new(
            cx(),
            config.clone(),
            Arc::new(CaptureParent {
                executor: executor(old.clone(), registry.clone()),
                handle: parent_handle.clone(),
            }),
            None,
        )
        .unwrap();
        let scheduler = owner.handle();
        let parent = scheduler
            .submit(Submission::new("worker", "parent"))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), old.entered.notified())
            .await
            .unwrap();
        let refreshed = Arc::new(registry.rebind(new_enabled.then(|| {
            Arc::new(RuntimeProgressAdapter::new(
                new_tracker.clone(),
                ProgressOptions::default(),
            )) as Arc<dyn RuntimeProgress>
        })));
        owner
            .reconfigure(
                config,
                Arc::new(executor(
                    Fake::new(vec![response(vec![message("nested")])]),
                    refreshed,
                )),
            )
            .await
            .unwrap();
        let scope = parent_handle.lock().unwrap().clone().unwrap();
        let child = scope
            .submit(Submission::new("worker", "after refresh"))
            .await
            .unwrap();
        let tasks = scheduler
            .wait(
                &[child],
                WaitMode::All,
                Some(std::time::Duration::from_secs(2)),
            )
            .await
            .unwrap();
        assert_eq!(tasks[0].status, TaskStatus::Completed);
        assert_eq!(
            old_tracker.snapshot().input_tokens,
            if old_enabled { 13 } else { 0 }
        );
        assert_eq!(new_tracker.snapshot().input_tokens, 0);
        assert_eq!(new_tracker.snapshot().agent_count, 0);
        old.release.notify_one();
        let tasks = scheduler
            .wait(
                &[parent],
                WaitMode::All,
                Some(std::time::Duration::from_secs(2)),
            )
            .await
            .unwrap();
        assert_eq!(tasks[0].status, TaskStatus::Completed);
        assert_eq!(
            old_tracker.snapshot().input_tokens,
            if old_enabled { 39 } else { 0 }
        );
        owner.shutdown().await.unwrap();
    }
}

impl CostEstimator for Cost {
    fn cost(&self, _: &str, _: &Usage) -> f64 {
        0.125
    }
}
#[tokio::test]
async fn actual_go_runtime_fixtures_cover_run_stream_and_nested_trackers() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/progress/runtime.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let depth = case["depth"].as_u64().unwrap();
        let failed = case["failed"].as_bool().unwrap();
        let streamed = case["streamed"].as_bool().unwrap();
        let root = ProgressTracker::new();
        let mut trackers = vec![root.clone()];
        for n in 0..depth {
            trackers.push(trackers.last().unwrap().child(format!("task-{n}")));
        }
        let progress = Arc::new(RuntimeProgressAdapter::new(
            trackers.last().unwrap().clone(),
            ProgressOptions {
                usd: UsdPolicy::HostUnitsAreUsd,
                ..Default::default()
            },
        ));
        let model = Arc::new(Fake {
            answers: Mutex::new(VecDeque::from([Ok(response(vec![message(
                "accepted text",
            )]))])),
            fail_stream: failed,
        });
        let runner = Runner::new(
            AgentConfig::new("agent", ModelBinding::streaming("configured-label", model)),
            RunnerConfig {
                progress: Some(progress),
                blocking_reasoning: true,
                cost_estimator: Some(Arc::new(Cost)),
                ..Default::default()
            },
        )
        .unwrap();
        let result = if streamed {
            runner
                .stream(cx(), request(), Arc::new(HostImpl))
                .finish()
                .await
        } else {
            runner.run(cx(), request(), Arc::new(HostImpl)).await
        };
        assert_eq!(result.is_err(), failed, "{}", case["name"]);
        for (tracker, expected) in trackers.iter().zip(case["expected"].as_array().unwrap()) {
            let snapshot = tracker.snapshot();
            let actual = json!({"input":snapshot.input_tokens,"output":snapshot.output_tokens,"cache_read":snapshot.cache_read_input_tokens,"cache_create":snapshot.cache_creation_input_tokens,"cost":snapshot.cost_usd,"models":snapshot.model_usage,"text":tracker.last_assistant_text()});
            let mut expected = expected.clone();
            expected["cost"] = json!(expected["cost"].as_f64().unwrap());
            assert_eq!(actual, expected, "{}", case["name"]);
        }
    }
}
fn frame() -> ProgressFrame<'static> {
    ProgressFrame {
        agent_name: "agent",
        agent_model: "primary",
        turn: 1,
        task_id: None,
    }
}
fn call(usage: &Usage, origin: UsageOrigin) -> ProgressUsage<'_> {
    ProgressUsage {
        origin,
        request_model: "fallback",
        provider: "provider",
        canonical_model: "canonical",
        usage,
        host_cost: Some(4.0),
        host_cost_known: true,
    }
}
#[test]
fn usd_is_explicit_and_conversion_failure_is_observational() {
    for (policy, expected) in [
        (UsdPolicy::Unpriced, 0.0),
        (UsdPolicy::HostUnitsAreUsd, 4.0),
        (
            UsdPolicy::Custom(Arc::new(|frame, call| {
                assert_eq!(frame.agent_model, "primary");
                assert_eq!(call.canonical_model, "canonical");
                Ok(0.25)
            })),
            0.25,
        ),
    ] {
        let tracker = ProgressTracker::new();
        let errors = Arc::new(Mutex::new(vec![]));
        let diagnostics = errors.clone();
        let adapter = RuntimeProgressAdapter::new(
            tracker.clone(),
            ProgressOptions {
                usd: policy,
                diagnostic: Some(Arc::new(move |error| {
                    diagnostics.lock().unwrap().push(error.to_owned())
                })),
                ..Default::default()
            },
        );
        adapter.usage(
            &cx(),
            frame(),
            call(
                &Usage {
                    input_tokens: 8,
                    ..Default::default()
                },
                UsageOrigin::Accepted,
            ),
        );
        assert_eq!(tracker.snapshot().cost_usd, expected);
        assert_eq!(
            tracker.snapshot().model_usage.unwrap()["primary"].input_tokens,
            8
        );
        adapter.usage(
            &cx(),
            frame(),
            call(
                &Usage {
                    input_tokens: u64::MAX,
                    ..Default::default()
                },
                UsageOrigin::Accepted,
            ),
        );
        assert_eq!(tracker.snapshot().input_tokens, 8);
        assert_eq!(errors.lock().unwrap().len(), 1);
        adapter.usage(
            &cx(),
            frame(),
            call(&Usage::default(), UsageOrigin::SummaryCompaction),
        );
        assert_eq!(tracker.snapshot().cost_usd, expected);
        adapter.usage(
            &cx(),
            frame(),
            call(
                &Usage {
                    input_tokens: 3,
                    ..Default::default()
                },
                UsageOrigin::ProviderCompaction,
            ),
        );
        assert_eq!(
            tracker.snapshot().model_usage.unwrap()["fallback"].input_tokens,
            3
        );
    }
}
#[derive(Default)]
struct Spans(Mutex<Vec<(bool, adk::tracewriter::Span)>>);
impl adk::tracing::TraceProcessor for Spans {
    fn span_start(&self, span: &adk::tracewriter::Span) {
        self.0.lock().unwrap().push((true, span.clone()));
    }
    fn span_end(&self, span: &adk::tracewriter::Span) {
        self.0.lock().unwrap().push((false, span.clone()));
    }
}
#[tokio::test]
async fn nested_executor_forks_from_immediate_parent_and_finishes_once() {
    use adk_runtime::{subagent::*, subagent_tools::ChildProgressRegistry};
    struct EmptyExecutor;
    impl ChildExecutor for EmptyExecutor {
        fn execute<'a>(
            &'a self,
            _: ChildInvocation,
            _: ChildControl,
        ) -> BoxFuture<'a, Result<ChildOutcome, Error>> {
            panic!("foreign executor must never run")
        }
    }
    let foreign = Scheduler::new(
        cx(),
        SchedulerConfig::default(),
        Arc::new(EmptyExecutor),
        None,
    )
    .unwrap();
    let foreign_session = Arc::new(SubagentSession::new(foreign.handle()));
    let spans = Arc::new(Spans::default());
    let tracker = ProgressTracker::new().with_tracing_processor(spans.clone());
    tracker.record_assistant_text("root activity");
    let adapter = Arc::new(RuntimeProgressAdapter::new(
        tracker.clone(),
        ProgressOptions {
            usd: UsdPolicy::HostUnitsAreUsd,
            ..Default::default()
        },
    ));
    let delegate = RunItem::ToolCall {
        call: ToolCall {
            id: "delegate".into(),
            name: "subagent".into(),
            arguments: json!({"message":"nested work", "mode":"sync"}),
            raw_arguments: None,
        },
    };
    let mut worker = AgentConfig::new(
        "worker",
        ModelBinding::complete(
            "worker-label",
            Fake::new(vec![
                response(vec![delegate]),
                response(vec![message("worker done")]),
            ]),
        ),
    );
    worker.tools = build_subagent_task_tools(foreign_session, "leaf");
    let config = || RunnerConfig {
        cost_estimator: Some(Arc::new(Cost)),
        progress: Some(adapter.clone()),
        ..Default::default()
    };
    let worker = Runner::new(worker, config()).unwrap();
    let leaf = Runner::new(
        AgentConfig::new(
            "leaf",
            ModelBinding::complete(
                "leaf-label",
                Fake::new(vec![response(vec![message("leaf done")])]),
            ),
        ),
        config(),
    )
    .unwrap();
    let executor = RunnerChildExecutor::new(
        [("worker".into(), worker), ("leaf".into(), leaf)]
            .into_iter()
            .collect(),
        Arc::new(HostImpl),
    )
    .with_progress(Arc::new(ChildProgressRegistry::new(adapter)));
    let owner = Scheduler::new(
        cx(),
        SchedulerConfig {
            max_concurrency: 1,
            agents: [
                ("worker".into(), SecurityBaseline::default()),
                ("leaf".into(), SecurityBaseline::default()),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        Arc::new(executor),
        None,
    )
    .unwrap();
    let handle = owner.handle();
    let task = handle
        .submit(Submission::new("worker", "delegate"))
        .await
        .unwrap();
    let tasks = handle
        .wait(&[task.clone()], WaitMode::All, None)
        .await
        .unwrap();
    assert_eq!(tasks[0].status, TaskStatus::Completed);
    assert_eq!(tracker.last_assistant_text(), "root activity");
    let snapshot = tracker.snapshot();
    assert_eq!(snapshot.input_tokens, 39);
    assert_eq!(snapshot.cost_usd, 0.375);
    assert_eq!(snapshot.tool_call_count, 1);
    assert_eq!(
        snapshot.agent_count, 1,
        "only the direct worker belongs to root activity"
    );
    assert!(tracker.agent_meta(&task).is_none());
    {
        let spans = spans.0.lock().unwrap();
        let ends: Vec<_> = spans
            .iter()
            .filter_map(|(start, span)| match &span.data {
                Some(adk::tracewriter::SpanData::Subagent(data)) if !start => Some(data),
                _ => None,
            })
            .collect();
        assert_eq!(ends.len(), 2);
        assert_eq!(
            ends.iter()
                .find(|data| data.subagent_type == "worker")
                .unwrap()
                .input_tokens,
            39
        );
        assert_eq!(
            ends.iter()
                .find(|data| data.subagent_type == "leaf")
                .unwrap()
                .input_tokens,
            13
        );
    }
    owner.shutdown().await.unwrap();
    foreign.shutdown().await.unwrap();
}
#[tokio::test]
async fn explicit_session_and_tracing_composition_have_one_structural_owner() {
    use adk::{tracewriter::SpanData, tracing::TraceSession, tracing_runtime::RunTrace};
    for ownership in [SpanOwnership::External, SpanOwnership::Tracker] {
        let spans = Arc::new(Spans::default());
        let tracker = ProgressTracker::new().with_tracing_processor(spans.clone());
        let adapter = Arc::new(RuntimeProgressAdapter::new(
            tracker.clone(),
            ProgressOptions {
                spans: ownership,
                ..Default::default()
            },
        ));
        let scope = adapter.session(7, "work");
        let trace = RunTrace::new(TraceSession::new("root", spans.clone()));
        let hooks = adapter.tracing_hooks(trace.observer());
        for observation in [
            Observation::CompactionStarted {
                context_tokens: 100,
                target_tokens: 20,
            },
            Observation::Compacted {
                before_items: 3,
                after_items: 1,
                context_tokens: 20,
            },
        ] {
            adapter.observe(&cx(), frame(), ProgressObservation::Lifecycle(&observation));
            hooks.observe(&cx(), observation).await.unwrap();
        }
        let outcome = Runner::new(
            AgentConfig::new(
                "agent",
                ModelBinding::complete("primary", Fake::new(vec![response(vec![message("once")])])),
            ),
            RunnerConfig {
                progress: Some(adapter),
                generation_observer: Some(trace.observer()),
                hooks: Some(hooks),
                ..Default::default()
            },
        )
        .unwrap()
        .run(cx(), request(), Arc::new(HostImpl))
        .await
        .unwrap();
        assert!(
            !tracker
                .snapshot()
                .events
                .iter()
                .any(|e| e.event_type == "session_complete")
        );
        scope.finish(&outcome.result, "completed");
        if ownership == SpanOwnership::External {
            trace.run_session(async move { Ok(outcome) }).await.unwrap();
        } else {
            trace.finish();
        }
        assert_eq!(tracker.last_assistant_text(), "once");
        let spans = spans.0.lock().unwrap();
        assert_eq!(
            spans
                .iter()
                .filter(|(start, span)| !start
                    && matches!(span.data, Some(SpanData::Compaction { .. })))
                .count(),
            1
        );
        assert_eq!(
            spans
                .iter()
                .filter(|(start, span)| !start && matches!(span.data, Some(SpanData::Session(_))))
                .count(),
            1
        );
    }
}
#[cfg(feature = "builder")]
#[tokio::test]
async fn automatic_owned_builder_binds_progress_without_root_activity_leak() {
    use adk::{builder::*, providers::factory::Kind};
    use adk_runtime::subagent::*;
    let tracker = ProgressTracker::new();
    tracker.record_assistant_text("root");
    let adapter = Arc::new(RuntimeProgressAdapter::new(
        tracker.clone(),
        ProgressOptions::default(),
    ));
    let model = Fake::new(vec![response(vec![message("child")])]);
    let mut bundle = Builder::new(Config {
        model: "openai/base".into(),
        roles: vec![RoleSpec {
            name: "worker".into(),
            instructions: "work".into(),
            tool_access: "read_only".into(),
            ..Default::default()
        }],
        features: Some(Features {
            subagents: SubagentFeatures {
                task: true,
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    })
    .model("openai", Kind::OpenAi, model)
    .unwrap()
    .runner_config(RunnerConfig {
        progress: Some(adapter),
        ..Default::default()
    })
    .subagent_host(Arc::new(HostImpl))
    .build(&cx())
    .await
    .unwrap();
    let session = bundle.session();
    let scheduler = &session.subagents().unwrap().scheduler;
    let id = scheduler
        .submit(Submission::new("worker", "work"))
        .await
        .unwrap();
    let tasks = scheduler.wait(&[id], WaitMode::All, None).await.unwrap();
    assert_eq!(tasks[0].status, TaskStatus::Completed);
    assert_eq!(tracker.snapshot().input_tokens, 13);
    assert_eq!(tracker.snapshot().agent_count, 1);
    assert_eq!(tracker.last_assistant_text(), "root");
    bundle.close().await.unwrap();
}
#[tokio::test]
async fn dropped_child_owner_cleans_scope_without_completion_usage() {
    use adk_runtime::{subagent::*, subagent_tools::ChildProgressRegistry};
    struct Waiting(Arc<tokio::sync::Notify>);
    impl Model for Waiting {
        fn provider(&self) -> &str {
            "provider"
        }
        fn complete<'a>(
            &'a self,
            _: &'a Context,
            _: ModelRequest,
        ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
            Box::pin(async {
                self.0.notify_one();
                std::future::pending().await
            })
        }
    }
    let entered = Arc::new(tokio::sync::Notify::new());
    let spans = Arc::new(Spans::default());
    let tracker = ProgressTracker::new().with_tracing_processor(spans.clone());
    let adapter = Arc::new(RuntimeProgressAdapter::new(
        tracker.clone(),
        ProgressOptions::default(),
    ));
    let runner = Runner::new(
        AgentConfig::new(
            "worker",
            ModelBinding::complete("model", Arc::new(Waiting(entered.clone()))),
        ),
        RunnerConfig::default(),
    )
    .unwrap();
    let registry = Arc::new(ChildProgressRegistry::new(adapter));
    let executor = RunnerChildExecutor::new(
        [("worker".into(), runner)].into_iter().collect(),
        Arc::new(HostImpl),
    )
    .with_progress(registry);
    let scheduler = Scheduler::new(
        cx(),
        SchedulerConfig {
            agents: [("worker".into(), SecurityBaseline::default())]
                .into_iter()
                .collect(),
            ..Default::default()
        },
        Arc::new(executor),
        None,
    )
    .unwrap();
    let task = scheduler
        .handle()
        .submit(Submission::new("worker", "work"))
        .await
        .unwrap();
    entered.notified().await;
    assert!(tracker.agent_meta(&task).is_some());
    drop(scheduler);
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while tracker.agent_meta(&task).is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(tracker.snapshot().input_tokens, 0);
    assert_eq!(tracker.snapshot().cost_usd, 0.0);
    let spans = spans.0.lock().unwrap();
    let ended: Vec<_> = spans.iter().filter(|(start, _)| !start).collect();
    assert_eq!(ended.len(), 1);
    let Some(adk::tracewriter::SpanData::Subagent(data)) = &ended[0].1.data else {
        panic!("subagent span")
    };
    assert_eq!(data.status, "stopped");
    assert_eq!(data.input_tokens, 0);
}
#[test]
fn resumed_task_scope_finishes_once_and_external_content_is_not_replayed() {
    let tracker = ProgressTracker::new();
    let adapter = RuntimeProgressAdapter::new(
        tracker.clone(),
        ProgressOptions {
            content: ContentOwnership::External,
            model_label: Some(Arc::new(|_, call| call.canonical_model.into())),
            ..Default::default()
        },
    );
    tracker.record_assistant_text("external accepted");
    adapter.observe(
        &cx(),
        frame(),
        ProgressObservation::Lifecycle(&Observation::ModelAccepted {
            agent: "agent".into(),
            response: response(vec![message("must not overwrite")]),
        }),
    );
    assert_eq!(tracker.last_assistant_text(), "external accepted");
    let child = ProgressChild {
        task_id: "resumed",
        parent_task_id: None,
        agent_name: "agent",
        agent_model: "primary",
    };
    for _ in 0..2 {
        let scoped = adapter.fork_child(child).unwrap();
        scoped.usage(
            &cx(),
            frame(),
            call(
                &Usage {
                    input_tokens: 2,
                    ..Default::default()
                },
                UsageOrigin::Accepted,
            ),
        );
        let end = adk_runtime::subagent::ChildOutcome::completed("done");
        adapter.child_finished(child, ProgressChildEnd::Finished(&end));
        adapter.child_finished(child, ProgressChildEnd::Finished(&end));
        assert!(tracker.agent_meta("resumed").is_none());
    }
    assert_eq!(tracker.snapshot().input_tokens, 4);
    assert_eq!(tracker.snapshot().agent_count, 2);
    assert_eq!(
        tracker.snapshot().model_usage.unwrap()["canonical"].input_tokens,
        4
    );
}

#[tokio::test]
async fn durable_child_resume_forks_once_without_recharging_checkpoint_usage() {
    use adk_runtime::{subagent::*, subagent_tools::ChildProgressRegistry};
    use std::sync::atomic::{AtomicBool, Ordering};
    #[derive(Default)]
    struct Journal {
        state: Mutex<Option<SchedulerCheckpoint>>,
        stop: AtomicBool,
        reached: tokio::sync::Notify,
    }
    impl SchedulerStore for Journal {
        fn persist<'a>(
            &'a self,
            _: &'a Context,
            state: &'a SchedulerCheckpoint,
        ) -> BoxFuture<'a, Result<(), Error>> {
            Box::pin(async move {
                *self.state.lock().unwrap() = Some(state.clone());
                if self.stop.load(Ordering::SeqCst)
                    && state.records.iter().any(|record| {
                        record
                            .durable_checkpoint
                            .as_ref()
                            .is_some_and(|checkpoint| {
                                checkpoint.execution_boundary() == "model_completed"
                            })
                    })
                {
                    self.reached.notify_one();
                    std::future::pending::<()>().await;
                }
                Ok(())
            })
        }
    }
    let tracker = ProgressTracker::new();
    let registry = Arc::new(ChildProgressRegistry::new(Arc::new(
        RuntimeProgressAdapter::new(tracker.clone(), ProgressOptions::default()),
    )));
    let first = RunItem::ToolCall {
        call: ToolCall {
            id: "missing".into(),
            name: "unknown".into(),
            arguments: json!({}),
            raw_arguments: None,
        },
    };
    let model = Fake::new(vec![
        response(vec![first]),
        response(vec![message("resumed")]),
    ]);
    let runner = Runner::new(
        AgentConfig::new("worker", ModelBinding::complete("primary", model)),
        RunnerConfig::default(),
    )
    .unwrap();
    let executor = Arc::new(
        RunnerChildExecutor::new(
            [("worker".into(), runner)].into_iter().collect(),
            Arc::new(HostImpl),
        )
        .with_progress(registry),
    );
    let journal = Arc::new(Journal {
        stop: AtomicBool::new(true),
        ..Default::default()
    });
    let config = SchedulerConfig {
        agents: [("worker".into(), SecurityBaseline::default())]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let owner = Scheduler::new(
        cx(),
        config.clone(),
        executor.clone(),
        Some(journal.clone()),
    )
    .unwrap();
    let id = owner
        .handle()
        .submit(Submission::new("worker", "work"))
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        journal.reached.notified(),
    )
    .await
    .unwrap();
    assert_eq!(tracker.snapshot().input_tokens, 13);
    let saved = journal.state.lock().unwrap().clone().unwrap();
    drop(owner);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while tracker.agent_meta(&id).is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    journal.stop.store(false, Ordering::SeqCst);
    let owner = Scheduler::new(cx(), config, executor, Some(journal)).unwrap();
    let handle = owner.handle();
    handle
        .restore(&cx(), serde_json::to_value(saved).unwrap())
        .await
        .unwrap();
    handle.resume_checkpoint(&id).await.unwrap();
    let tasks = handle
        .wait(
            &[id.clone()],
            WaitMode::All,
            Some(std::time::Duration::from_secs(2)),
        )
        .await
        .unwrap();
    assert_eq!(tasks[0].status, TaskStatus::Completed);
    assert_eq!(tasks[0].result, "resumed");
    assert_eq!(tracker.snapshot().input_tokens, 26);
    assert_eq!(tracker.snapshot().agent_count, 2);
    assert!(tracker.agent_meta(&id).is_none());
    owner.shutdown().await.unwrap();
}

#[test]
fn restored_ancestor_bindings_do_not_replay_spans_and_are_reused_on_start() {
    let spans = Arc::new(Spans::default());
    let tracker = ProgressTracker::new().with_tracing_processor(spans.clone());
    let root = RuntimeProgressAdapter::new(tracker.clone(), ProgressOptions::default());
    let parent = ProgressChild {
        task_id: "parent",
        parent_task_id: None,
        agent_name: "worker",
        agent_model: "",
    };
    let middle = ProgressChild {
        task_id: "middle",
        parent_task_id: Some("parent"),
        ..parent
    };
    let leaf = ProgressChild {
        task_id: "leaf",
        parent_task_id: Some("middle"),
        ..parent
    };
    let parent_binding = root.restore_child(parent).unwrap();
    let middle_binding = parent_binding.restore_child(middle).unwrap();
    assert!(spans.0.lock().unwrap().is_empty());
    assert_eq!(tracker.snapshot().agent_count, 0);
    assert_eq!(tracker.snapshot().input_tokens, 0);
    let leaf_binding = middle_binding.fork_child(leaf).unwrap();
    let usage = Usage {
        input_tokens: 2,
        ..Default::default()
    };
    leaf_binding.usage(&cx(), frame(), call(&usage, UsageOrigin::Accepted));
    assert_eq!(tracker.snapshot().input_tokens, 2);
    assert_eq!(spans.0.lock().unwrap().len(), 1, "only leaf starts");
    let running_parent = root.fork_child(parent).unwrap();
    let running_middle = parent_binding.fork_child(middle).unwrap();
    assert!(Arc::ptr_eq(&parent_binding, &running_parent));
    assert!(Arc::ptr_eq(&middle_binding, &running_middle));
    leaf_binding.usage(&cx(), frame(), call(&usage, UsageOrigin::Accepted));
    let outcome = adk_runtime::subagent::ChildOutcome::completed("done");
    middle_binding.child_finished(leaf, ProgressChildEnd::Finished(&outcome));
    parent_binding.child_finished(middle, ProgressChildEnd::Finished(&outcome));
    root.child_finished(parent, ProgressChildEnd::Finished(&outcome));
    assert_eq!(tracker.snapshot().input_tokens, 4);
    let spans = spans.0.lock().unwrap();
    assert_eq!(spans.len(), 6);
    for (_, span) in spans.iter().filter(|(start, _)| !start) {
        let Some(adk::tracewriter::SpanData::Subagent(data)) = &span.data else {
            panic!("subagent span")
        };
        assert_eq!(
            data.input_tokens, 4,
            "live usage reaches every real immediate parent"
        );
    }
}
