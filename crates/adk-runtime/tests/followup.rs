use adk_codec::config::{RunConfigSentinels, ToolPolicySentinels};
use adk_core::*;
use adk_runtime::{compaction::*, compat::*, *};
use serde_json::json;
use std::{
    collections::VecDeque,
    num::NonZeroU32,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn message(role: Role, text: impl Into<String>) -> RunItem {
    RunItem::Message {
        message: Message {
            role,
            content: vec![Content::Text { text: text.into() }],
        },
    }
}
fn response(items: Vec<RunItem>, end: bool) -> ModelResponse {
    ModelResponse {
        items,
        usage: Usage::default(),
        end_turn: Some(end),
        response_id: None,
        metadata: Default::default(),
    }
}
fn answer() -> ModelResponse {
    response(vec![message(Role::Assistant, "done")], true)
}
fn context() -> Context {
    Context {
        run_id: "followup".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn request(input: Vec<RunItem>, turns: u32) -> RunRequest {
    RunRequest {
        input,
        policy: RunPolicy {
            max_turns: NonZeroU32::new(turns).unwrap(),
            ..Default::default()
        },
    }
}
struct Script {
    replies: Mutex<VecDeque<Result<ModelResponse, Error>>>,
    requests: Mutex<Vec<ModelRequest>>,
}
impl Script {
    fn new(replies: Vec<Result<ModelResponse, Error>>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into()),
            requests: Mutex::new(vec![]),
        })
    }
}
impl Model for Script {
    fn provider(&self) -> &str {
        "fake"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected dispatch")
        })
    }
}
struct Events(Option<ModelResponse>);
impl ModelStream for Events {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async {
            Ok(self
                .0
                .take()
                .map(|response| ModelEvent::Complete { response }))
        })
    }
}
impl StreamingModel for Script {
    fn stream<'a>(
        &'a self,
        ctx: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            let response = self.complete(ctx, request).await?;
            Ok(Box::new(Events(Some(response))) as Box<dyn ModelStream>)
        })
    }
}
struct Quiet;
impl Host for Quiet {
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
#[derive(Default)]
struct Hooks {
    journal: ApprovalJournal,
    events: Mutex<Vec<Observation>>,
}
impl RunHooks for Hooks {
    fn observe<'a>(
        &'a self,
        context: &'a Context,
        event: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.journal.observe(context, event.clone()).await?;
            self.events.lock().unwrap().push(event);
            Ok(())
        })
    }
}
#[test]
fn model_thresholds_match_executed_go() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/compaction.json")).unwrap();
    for (model, expected) in fixture["model_thresholds"].as_object().unwrap() {
        let policy = LocalCompactionPolicy::for_model(model);
        assert_eq!(
            json!([policy.trigger_tokens, policy.target_tokens]),
            *expected,
            "{model}"
        );
    }
}
#[tokio::test]
async fn default_threshold_selection_changes_first_request_without_provider_usage() {
    for (name, should_compact) in [("provider/gpt-6-mini", true), ("provider/gpt-6", false)] {
        let model = Script::new(vec![Ok(answer())]);
        let runner = Runner::new(
            AgentConfig::new("agent", ModelBinding::complete(name, model.clone())),
            RunnerConfig::default(),
        )
        .unwrap();
        let history = vec![
            message(Role::User, "task"),
            message(Role::Assistant, "obsolete ".repeat(60000)),
            message(Role::Assistant, "recent"),
        ];
        runner
            .run(context(), request(history.clone(), 1), Arc::new(Quiet))
            .await
            .unwrap();
        let sent = &model.requests.lock().unwrap()[0].input;
        assert_eq!(!extract_summary(sent).is_empty(), should_compact);
        if !should_compact {
            assert_eq!(*sent, history);
        }
    }
}
#[tokio::test]
async fn forced_overflow_compacts_once_retries_same_model_and_preserves_transient_cache_and_budget()
{
    for streaming in [false, true] {
        for turns in [1, 2] {
            let model = Script::new(vec![
                Err(Error::new(
                    ErrorCategory::Provider,
                    "CONTEXT_LENGTH_EXCEEDED",
                )),
                Ok(answer()),
            ]);
            let binding = if streaming {
                ModelBinding::streaming("fake", model.clone())
            } else {
                ModelBinding::complete("fake", model.clone())
            };
            let hooks = Arc::new(Hooks::default());
            let transient = message(Role::Developer, "private turn-only hint");
            let config = RunnerConfig {
                transient_context: vec![transient.clone()],
                hooks: Some(hooks.clone()),
                prompt_cache_key: Some("key".into()),
                ..Default::default()
            };
            let runner = Runner::new(AgentConfig::new("agent", binding), config).unwrap();
            let history = vec![
                message(Role::User, "task"),
                message(Role::Assistant, "obsolete content ".repeat(5000)),
                message(Role::Assistant, "latest"),
            ];
            let outcome = if streaming {
                runner
                    .stream(context(), request(history.clone(), turns), Arc::new(Quiet))
                    .finish()
                    .await
            } else {
                runner
                    .run(context(), request(history.clone(), turns), Arc::new(Quiet))
                    .await
            };
            let result = if turns == 1 {
                let error = outcome.err().unwrap();
                assert_eq!(error.error.info.category, ErrorCategory::MaxTurns);
                *error.partial.unwrap()
            } else {
                outcome.unwrap().result
            };
            assert!(!extract_summary(&result.history).is_empty());
            assert!(!result.history.contains(&transient));
            assert_eq!(result.new_items.len(), usize::from(turns == 2));
            let requests = model.requests.lock().unwrap();
            assert_eq!(requests.len(), turns as usize);
            assert_eq!(&requests[0].input[..history.len()], &history);
            if turns == 2 {
                assert_eq!(requests[0].settings, requests[1].settings);
                assert_eq!(requests[1].input.last(), Some(&transient));
                assert!(!extract_summary(&requests[1].input).is_empty());
            }
            assert_eq!(
                hooks
                    .events
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|event| matches!(event, Observation::Compacted { .. }))
                    .count(),
                1
            );
        }
    }
}
#[tokio::test]
async fn forced_overflow_noop_or_disabled_does_not_spin_or_retry() {
    for disabled in [false, true] {
        let model = Script::new(vec![Err(Error::new(
            ErrorCategory::Provider,
            "exceeds the context window",
        ))]);
        let config = RunnerConfig {
            local_compaction: LocalCompactionPolicy {
                enabled: !disabled,
                ..Default::default()
            },
            ..Default::default()
        };
        let runner = Runner::new(
            AgentConfig::new("agent", ModelBinding::complete("fake", model.clone())),
            config,
        )
        .unwrap();
        let error = runner
            .run(
                context(),
                request(vec![message(Role::User, "task")], 4),
                Arc::new(Quiet),
            )
            .await
            .err()
            .unwrap();
        assert_eq!(error.error.info.category, ErrorCategory::Provider);
        assert_eq!(model.requests.lock().unwrap().len(), 1);
    }
}
struct TestTool {
    definition: ToolDefinition,
    control: bool,
    pending: bool,
    pause: bool,
    calls: AtomicUsize,
}
impl TestTool {
    fn new(name: &str, read_only: bool, control: bool, approval: bool, pending: bool) -> Arc<Self> {
        Arc::new(Self {
            definition: ToolDefinition {
                name: name.into(),
                description: String::new(),
                input_schema: schemars::json_schema!({}),
                read_only,
                requires_approval: approval,
            },
            control,
            pending,
            pause: false,
            calls: AtomicUsize::new(0),
        })
    }
}
impl Tool for TestTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn is_control_flow(&self) -> bool {
        self.control
    }
    fn timeout(&self) -> Option<Duration> {
        self.pending.then_some(Duration::from_millis(2))
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.pending {
                return std::future::pending().await;
            }
            Ok(ToolOutput {
                content: vec![Content::Text { text: "ok".into() }],
                is_error: false,
                should_pause: self.pause,
            })
        })
    }
}
fn call(name: &str) -> RunItem {
    RunItem::ToolCall {
        call: ToolCall {
            id: name.into(),
            name: name.into(),
            arguments: json!({}),
        },
    }
}
#[tokio::test]
async fn mutation_only_policy_preserves_read_control_own_approval_and_denial() {
    let tools = [
        TestTool::new("read", true, false, false, false),
        TestTool::new("write", false, false, false, false),
        TestTool::new("control", false, true, false, false),
        TestTool::new("own", true, false, true, false),
        TestTool::new("denied", false, false, false, false),
    ];
    let model = Script::new(vec![Ok(response(
        tools
            .iter()
            .map(|tool| call(&tool.definition.name))
            .collect(),
        false,
    ))]);
    let mut agent = AgentConfig::new("agent", ModelBinding::complete("fake", model));
    agent.tools = tools
        .iter()
        .map(|tool| tool.clone() as Arc<dyn Tool>)
        .collect();
    let mut config = RunnerConfig::default();
    let mut req = request(vec![], 2);
    req.policy.tools.access = AccessMode::WorkspaceWrite;
    req.policy.tools.denied_tools.insert("denied".into());
    apply_go_config(
        &RunConfigSentinels {
            tool_policy: Some(ToolPolicySentinels {
                approval_required: true,
                default_timeout: 0,
            }),
            ..Default::default()
        },
        &mut config,
        &mut req.policy,
    )
    .unwrap();
    let outcome = Runner::new(agent, config)
        .unwrap()
        .run(context(), req, Arc::new(Quiet))
        .await
        .unwrap();
    assert_eq!(
        outcome
            .result
            .pending_approvals
            .iter()
            .map(|r| r.call.name.as_str())
            .collect::<Vec<_>>(),
        ["write", "own"]
    );
    assert_eq!(
        tools
            .iter()
            .map(|t| t.calls.load(Ordering::SeqCst))
            .collect::<Vec<_>>(),
        [1, 0, 1, 0, 0]
    );
}
#[tokio::test]
async fn own_tool_timeout_is_model_visible_and_host_override_wins() {
    for limit in [None, Some(Duration::from_millis(4))] {
        let tool = TestTool::new("wait", true, false, false, true);
        let model = Script::new(vec![Ok(response(vec![call("wait")], false)), Ok(answer())]);
        let mut agent = AgentConfig::new("agent", ModelBinding::complete("fake", model));
        agent.tools = vec![tool];
        let mut req = request(vec![], 2);
        req.policy.tools.timeout = limit;
        let outcome = Runner::new(agent, RunnerConfig::default())
            .unwrap()
            .run(context(), req, Arc::new(Quiet))
            .await
            .unwrap();
        let output = outcome
            .result
            .new_items
            .iter()
            .find_map(|item| match item {
                RunItem::ToolResult { output, .. } => Some(output),
                _ => None,
            })
            .unwrap();
        assert!(output.is_error);
        assert_eq!(
            output.content,
            vec![Content::Text {
                text: format!(
                    "tool \"wait\" timed out after {}ms",
                    if limit.is_some() { 4 } else { 2 }
                )
            }]
        );
    }
}
struct Gate;
impl GoApprovalGate for Gate {
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: &'a ApprovalRequest,
    ) -> BoxFuture<'a, Result<GoApprovalDecision, Error>> {
        Box::pin(async {
            Ok(GoApprovalDecision {
                approved: true,
                reason: String::new(),
            })
        })
    }
}
#[tokio::test]
async fn automatic_compaction_uses_and_replaces_marker_journal_without_rewriting_new_items() {
    let model = Script::new(vec![
        Ok(response(vec![call("approved")], false)),
        Ok(response(
            vec![
                message(Role::Assistant, "obsolete ".repeat(10000)),
                message(Role::Assistant, "latest"),
            ],
            false,
        )),
        Ok(answer()),
    ]);
    let mut agent = AgentConfig::new("agent", ModelBinding::complete("fake", model.clone()));
    let tool = TestTool::new("approved", true, false, true, false);
    agent.tools = vec![tool.clone()];
    let hooks = Arc::new(Hooks::default());
    let policy = LocalCompactionPolicy {
        trigger_tokens: 27000,
        target_tokens: 26000,
        preserve_recent_items: 1,
        preserve_initial_user_messages: 1,
        ..Default::default()
    };
    let runner = Runner::new(
        agent,
        RunnerConfig {
            hooks: Some(hooks.clone()),
            local_compaction: policy,
            ..Default::default()
        },
    )
    .unwrap();
    let outcome = run_go_chat(
        &runner,
        context(),
        request(vec![message(Role::User, "task")], 3),
        Arc::new(Quiet),
        &Gate,
        None,
    )
    .await
    .unwrap();
    assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
    assert_eq!(hooks.journal.entries().len(), 2);
    assert!(hooks.journal.history_markers().unwrap().is_empty());
    assert_eq!(outcome.result.new_items.len(), 5);
    assert!(!extract_summary(&outcome.result.history).is_empty());
    let events = hooks.events.lock().unwrap();
    let (before, after, markers) = events
        .iter()
        .find_map(|event| match event {
            Observation::ApprovalHistoryReplaced {
                before,
                after,
                markers,
            } => Some((before, after, markers)),
            _ => None,
        })
        .expect("automatic compaction event");
    let old_markers = hooks
        .journal
        .entries()
        .iter()
        .map(|entry| adk_codec::approval::ApprovalMarkerBoundary {
            before_item: 2,
            marker: entry.marker.clone(),
        })
        .collect::<Vec<_>>();
    let sent = model.requests.lock().unwrap();
    let expected = compact_with_approvals(
        before,
        &old_markers,
        policy,
        estimate_request_overhead_tokens(&sent[2]) as i64,
    );
    let (expected_history, expected_markers) = finalize_local_history_with_approvals(
        &expected.history,
        &expected.markers,
        before,
        &old_markers,
    );
    assert_eq!(*after, expected_history);
    assert_eq!(*markers, expected_markers);
}

struct BlockedSink {
    active: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
    entered: tokio::sync::Notify,
}
struct SinkGuard(Arc<AtomicUsize>, Arc<AtomicUsize>);
impl Drop for SinkGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
        self.1.fetch_add(1, Ordering::SeqCst);
    }
}
impl GoEventSink for BlockedSink {
    fn emit<'a>(&'a self, _: &'a Context, _: GoStreamEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.active.fetch_add(1, Ordering::SeqCst);
            let _guard = SinkGuard(self.active.clone(), self.dropped.clone());
            self.entered.notify_one();
            std::future::pending().await
        })
    }
}
#[tokio::test]
async fn wire_event_sink_backpressure_stops_dispatch_and_owner_drop_closes_it() {
    let tool = TestTool::new("read", true, false, false, false);
    let model = Script::new(vec![Ok(response(vec![call("read")], false)), Ok(answer())]);
    let mut agent = AgentConfig::new("agent", ModelBinding::streaming("fake", model.clone()));
    agent.tools = vec![tool.clone()];
    let sink = Arc::new(BlockedSink {
        active: Arc::new(AtomicUsize::new(0)),
        dropped: Arc::new(AtomicUsize::new(0)),
        entered: tokio::sync::Notify::new(),
    });
    let runner = Runner::new(
        agent,
        RunnerConfig {
            hooks: Some(Arc::new(GoEventAdapter(sink.clone()))),
            ..Default::default()
        },
    )
    .unwrap();
    let mut stream = runner.stream(context(), request(vec![], 2), Arc::new(Quiet));
    assert!(matches!(
        stream.next().await,
        Some(RunEvent::Started { .. })
    ));
    // A raw complete event can precede accepted-item publication.
    assert!(matches!(
        stream.next().await,
        Some(RunEvent::Model {
            event: ModelEvent::Complete { .. }
        })
    ));
    let mut next = Box::pin(stream.next());
    tokio::select! {
        _ = sink.entered.notified() => {},
        value = &mut next => panic!("sink did not apply backpressure: {value:?}"),
    }
    assert_eq!(sink.active.load(Ordering::SeqCst), 1);
    assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    drop(next);
    drop(stream);
    assert_eq!(sink.active.load(Ordering::SeqCst), 0);
    assert_eq!(sink.dropped.load(Ordering::SeqCst), 1);
}

#[derive(Default)]
struct WireEvents(Mutex<Vec<GoStreamEvent>>);
impl GoEventSink for WireEvents {
    fn emit<'a>(
        &'a self,
        _: &'a Context,
        event: GoStreamEvent,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(event);
            Ok(())
        })
    }
}
#[tokio::test]
async fn wire_bridge_preserves_pause_continuation_and_projects_handoff_outputs_in_call_order() {
    let wire = Arc::new(WireEvents::default());
    let mut tool = TestTool::new("pause", true, false, false, false);
    Arc::get_mut(&mut tool).unwrap().pause = true;
    let model = Script::new(vec![Ok(response(vec![call("pause")], false)), Ok(answer())]);
    let mut agent = AgentConfig::new("source", ModelBinding::complete("fake", model));
    agent.tools = vec![tool.clone()];
    let runner = Runner::new(
        agent,
        RunnerConfig {
            hooks: Some(Arc::new(GoEventAdapter(wire.clone()))),
            ..Default::default()
        },
    )
    .unwrap();
    let paused = runner
        .run(context(), request(vec![], 2), Arc::new(Quiet))
        .await
        .unwrap();
    assert_eq!(paused.result.status, RunStatus::Paused);
    assert!(
        paused
            .result
            .new_items
            .iter()
            .any(|item| matches!(item, RunItem::ToolResult { output, .. } if output.should_pause))
    );
    let done = paused.continuation.unwrap().resume(None).await.unwrap();
    assert_eq!(done.result.status, RunStatus::Completed);
    assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
    assert!(wire.0.lock().unwrap().iter().any(|event| matches!(event, GoStreamEvent::Item(item) if item.tool_output.as_ref().is_some_and(|output| output.call_id == "pause"))));

    let wire = Arc::new(WireEvents::default());
    let target_model = Script::new(vec![Ok(answer())]);
    let target = Arc::new(AgentConfig::new(
        "target",
        ModelBinding::complete("target", target_model.clone()),
    ));
    let source_model = Script::new(vec![Ok(response(
        vec![call("before"), call("transfer"), call("after")],
        false,
    ))]);
    let mut source = AgentConfig::new("source", ModelBinding::complete("source", source_model));
    let before = TestTool::new("before", true, false, false, false);
    let after = TestTool::new("after", true, false, false, false);
    source.tools = vec![before.clone(), after.clone()];
    source.handoffs = vec![Handoff {
        definition: TestTool::new("transfer", true, true, false, false)
            .definition
            .clone(),
        target,
    }];
    let runner = Runner::new(
        source,
        RunnerConfig {
            hooks: Some(Arc::new(GoEventAdapter(wire.clone()))),
            ..Default::default()
        },
    )
    .unwrap();
    let done = runner
        .run(context(), request(vec![], 2), Arc::new(Quiet))
        .await
        .unwrap();
    assert_eq!(done.result.last_agent.as_deref(), Some("target"));
    assert_eq!(
        before.calls.load(Ordering::SeqCst) + after.calls.load(Ordering::SeqCst),
        0
    );
    assert_eq!(target_model.requests.lock().unwrap().len(), 1);
    let events = wire.0.lock().unwrap();
    let outputs: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            GoStreamEvent::Item(item) => item.tool_output.as_ref().map(|output| {
                (
                    item.agent_name.as_str(),
                    output.call_id.as_str(),
                    output.content.as_str(),
                    output.is_error,
                )
            }),
            _ => None,
        })
        .collect();
    assert_eq!(
        outputs,
        [
            (
                "source",
                "before",
                "not executed: the conversation was handed off to target in this turn",
                true
            ),
            ("source", "transfer", "Handing off to target", false),
            (
                "source",
                "after",
                "not executed: the conversation was handed off to target in this turn",
                true
            )
        ]
    );
}
struct RejectTwice(AtomicUsize);
impl adk_runtime::runner::StopGate for RejectTwice {
    fn check<'a>(
        &'a self,
        _: &'a Context,
        _: &'a serde_json::Value,
    ) -> BoxFuture<'a, Result<Option<String>, Error>> {
        Box::pin(
            async move { Ok((self.0.fetch_add(1, Ordering::SeqCst) < 2).then(|| "verify".into())) },
        )
    }
}
#[tokio::test]
async fn stop_gate_resets_on_tools_and_explicit_nonfinal_progress() {
    for tools in [false, true] {
        let middle = |id: &str| {
            if tools {
                response(
                    vec![RunItem::ToolCall {
                        call: ToolCall {
                            id: id.into(),
                            name: "read".into(),
                            arguments: json!({}),
                        },
                    }],
                    false,
                )
            } else {
                response(vec![message(Role::Assistant, "working")], false)
            }
        };
        let model = Script::new(vec![
            Ok(answer()),
            Ok(middle("first")),
            Ok(answer()),
            Ok(middle("second")),
            Ok(answer()),
        ]);
        let gate = Arc::new(RejectTwice(AtomicUsize::new(0)));
        let mut agent = AgentConfig::new("agent", ModelBinding::complete("fake", model));
        agent.tools = vec![TestTool::new("read", true, false, false, false)];
        let runner = Runner::new(
            agent,
            RunnerConfig {
                stop_gate: Some(gate.clone()),
                stop_gate_max_blocks: 2,
                ..Default::default()
            },
        )
        .unwrap();
        let result = runner
            .run(context(), request(vec![], 6), Arc::new(Quiet))
            .await
            .unwrap();
        assert_eq!(result.result.status, RunStatus::Completed);
        assert_eq!(
            gate.0.load(Ordering::SeqCst),
            3,
            "gate bypassed after nonconsecutive blocks"
        );
    }
}
struct ReplaceOld;
impl Compactor for ReplaceOld {
    fn compact<'a>(
        &'a self,
        _: &'a Context,
        mut req: CompactionRequest,
    ) -> BoxFuture<'a, Result<CompactedHistory, Error>> {
        Box::pin(async move {
            req.history[2] = message(Role::Assistant, "summary");
            Ok(CompactedHistory {
                history: req.history,
                context_tokens: 10,
                usage: Usage::default(),
                cost: 0.0,
            })
        })
    }
}
#[tokio::test]
async fn custom_compaction_rebases_approval_anchors_despite_repeated_ordinary_messages() {
    let mut first = response(vec![call("approved")], false);
    first.usage.context_tokens = Some(100);
    let model = Script::new(vec![Ok(first), Ok(answer())]);
    let mut agent = AgentConfig::new("agent", ModelBinding::complete("fake", model.clone()));
    agent.tools = vec![TestTool::new("approved", true, false, true, false)];
    let hooks = Arc::new(Hooks::default());
    let runner = Runner::new(
        agent,
        RunnerConfig {
            hooks: Some(hooks.clone()),
            local_compaction: LocalCompactionPolicy {
                enabled: false,
                ..Default::default()
            },
            compaction: Some(CompactionConfig {
                trigger_tokens: 50,
                target_tokens: 10,
                compactor: Arc::new(ReplaceOld),
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let history = vec![
        message(Role::User, "continue"),
        message(Role::User, "continue"),
        message(Role::Assistant, "old"),
    ];
    let result = run_go_chat(
        &runner,
        context(),
        request(history, 2),
        Arc::new(Quiet),
        &Gate,
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.result.status, RunStatus::Completed);
    assert_eq!(
        model.requests.lock().unwrap()[1].input[2],
        message(Role::Assistant, "summary")
    );
    assert_eq!(
        hooks
            .journal
            .history_markers()
            .unwrap()
            .iter()
            .map(|marker| marker.before_item)
            .collect::<Vec<_>>(),
        [4, 4]
    );
}
