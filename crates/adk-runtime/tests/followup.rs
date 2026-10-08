use adk_codec::config::{RunConfigSentinels, ToolPolicySentinels};
use adk_core::*;
use adk_runtime::{compaction::*, compat::*, *};
use serde_json::{Value, json};
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
        snapshot_raw: None,
        snapshot_projection: None,
        raw: None,
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
        input_provenance: Vec::new(),
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
            RunnerConfig {
                local_compaction: LocalCompactionPolicy {
                    use_llm_summary: false,
                    ..Default::default()
                },
                ..Default::default()
            },
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
                local_compaction: LocalCompactionPolicy {
                    use_llm_summary: false,
                    ..Default::default()
                },
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
        use_llm_summary: false,
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
        on_handoff: None,
        is_enabled: None,
        input_filter: Default::default(),
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
            req.history_provenance[2] = ItemProvenance::Unattributed;
            Ok(CompactedHistory {
                history_provenance: req.history_provenance,
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

#[tokio::test]
async fn handoff_skipped_siblings_preserve_but_do_not_advance_tool_error_streak() {
    let target_model = Script::new(vec![
        Ok(response(vec![call("missing3")], false)),
        Ok(answer()),
    ]);
    let target = Arc::new(AgentConfig::new(
        "target",
        ModelBinding::complete("target", target_model.clone()),
    ));
    let source_model = Script::new(vec![
        Ok(response(vec![call("missing1")], false)),
        Ok(response(vec![call("missing2")], false)),
        Ok(response(vec![call("skip"), call("transfer")], false)),
    ]);
    let mut source = AgentConfig::new("source", ModelBinding::complete("source", source_model));
    let skipped = TestTool::new("skip", true, false, false, false);
    source.tools = vec![skipped.clone()];
    source.handoffs = vec![Handoff {
        on_handoff: None,
        is_enabled: None,
        input_filter: Default::default(),
        definition: TestTool::new("transfer", true, true, false, false)
            .definition
            .clone(),
        target,
    }];
    let runner = Runner::new(source, RunnerConfig::default()).unwrap();
    let outcome = runner
        .run(context(), request(vec![], 5), Arc::new(Quiet))
        .await
        .unwrap();
    let escalation = |items: &[RunItem]| {
        items.iter().filter(|item| matches!(item, RunItem::Message { message } if message.content.iter().any(|content| matches!(content, Content::Text { text } if text.starts_with("[SYSTEM] Your last 3 tool turns all failed."))))).count()
    };
    let sent = target_model.requests.lock().unwrap();
    assert_eq!(
        escalation(&sent[0].input),
        0,
        "successful handoff advanced the error streak"
    );
    assert_eq!(
        escalation(&sent[1].input),
        1,
        "successful handoff reset the existing error streak"
    );
    assert_eq!(escalation(&outcome.result.new_items), 1);
    assert_eq!(skipped.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stop_gate_does_not_run_when_all_handoffs_are_denied() {
    let model = Script::new(vec![Ok(answer())]);
    let target_model = Script::new(vec![]);
    let target = Arc::new(AgentConfig::new(
        "target",
        ModelBinding::complete("target", target_model.clone()),
    ));
    let mut agent = AgentConfig::new("source", ModelBinding::complete("source", model.clone()));
    agent.handoffs = vec![Handoff {
        on_handoff: None,
        is_enabled: None,
        input_filter: Default::default(),
        definition: TestTool::new("transfer", true, true, false, false)
            .definition
            .clone(),
        target,
    }];
    let gate = Arc::new(RejectTwice(AtomicUsize::new(0)));
    let runner = Runner::new(
        agent,
        RunnerConfig {
            stop_gate: Some(gate.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let mut req = request(vec![], 1);
    req.policy.tools.denied_tools.insert("transfer".into());
    let outcome = runner.run(context(), req, Arc::new(Quiet)).await.unwrap();
    assert_eq!(outcome.result.final_output, Some(json!("done")));
    assert_eq!(gate.0.load(Ordering::SeqCst), 0);
    assert!(model.requests.lock().unwrap()[0].tools.is_empty());
    assert!(target_model.requests.lock().unwrap().is_empty());
}

#[test]
fn model_threshold_names_match_pinned_sdk_simple_lowercase_and_precedence() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/metadata-compaction/observations.json"
    ))
    .unwrap();
    let cases = fixture["static_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 45);
    for case in cases {
        let model = case["model"].as_str().unwrap();
        let policy = LocalCompactionPolicy::for_model(model);
        assert_eq!(
            json!([policy.trigger_tokens, policy.target_tokens]),
            json!([case["trigger"], case["target"]]),
            "{model}"
        );
    }
}

struct ConfirmationGate(AtomicUsize);
impl StopGate for ConfirmationGate {
    fn check<'a>(
        &'a self,
        _: &'a Context,
        _: &'a serde_json::Value,
    ) -> BoxFuture<'a, Result<Option<String>, Error>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Some("check".into()))
        })
    }
}
struct ConfirmationInput {
    calls: AtomicUsize,
    at: usize,
}
impl ConfirmationInput {
    fn take(&self) -> ImmediateInputBatch {
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.at {
            ImmediateInputBatch {
                items: vec![message(Role::User, "steering")],
                provenance: vec![ItemProvenance::Unattributed],
            }
        } else {
            ImmediateInputBatch::default()
        }
    }
}
impl ImmediateInputPoller for ConfirmationInput {
    fn poll<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<ImmediateInputBatch, Error>> {
        Box::pin(async move { Ok(self.take()) })
    }
}
impl ImmediateInputFinalizer for ConfirmationInput {
    fn finalize<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<ImmediateInputBatch, Error>> {
        Box::pin(async move { Ok(self.take()) })
    }
}

#[tokio::test]
async fn completion_confirmation_matches_pinned_normal_and_streamed_sdk() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/confirmation/observations.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let streaming = case["streamed"].as_bool().unwrap();
        let reply = |text: &str| response(vec![message(Role::Assistant, text)], true);
        let mut turns = 1;
        let replies = match name {
            "tools" => {
                turns = 3;
                vec![
                    reply("first"),
                    response(vec![call("read")], true),
                    reply("again"),
                    reply("final"),
                ]
            }
            "end_turn" => {
                turns = 3;
                vec![
                    reply("first"),
                    response(vec![message(Role::Assistant, "progress")], false),
                    reply("again"),
                    reply("final"),
                ]
            }
            "gate" | "poll" => vec![reply("first"), reply("again"), reply("final")],
            "finalize" => vec![
                reply("first"),
                reply("again"),
                reply("after input"),
                reply("final"),
            ],
            _ => vec![reply("first"), reply("final")],
        };
        let model = Script::new(replies.into_iter().map(Ok).collect());
        let binding = if streaming {
            ModelBinding::streaming("fake", model.clone())
        } else {
            ModelBinding::complete("fake", model.clone())
        };
        let mut agent = AgentConfig::new("agent", binding);
        if name != "no_tools" {
            agent
                .tools
                .push(TestTool::new("read", true, false, false, false));
        }
        let gate = Arc::new(ConfirmationGate(AtomicUsize::new(0)));
        let poller = Arc::new(ConfirmationInput {
            calls: AtomicUsize::new(0),
            at: 2,
        });
        let finalizer = Arc::new(ConfirmationInput {
            calls: AtomicUsize::new(0),
            at: 1,
        });
        let mut config = RunnerConfig {
            require_completion_confirmation: name != "off",
            force_final_summary_turn: name == "summary",
            ..Default::default()
        };
        if name == "gate" {
            config.stop_gate = Some(gate.clone());
            config.stop_gate_max_blocks = 1;
        }
        if name == "poll" {
            config.immediate_input_poller = Some(poller.clone());
        }
        if name == "finalize" {
            config.immediate_input_finalizer = Some(finalizer.clone());
        }
        let runner = Runner::new(agent, config).unwrap();
        let outcome = if streaming {
            runner
                .stream(context(), request(vec![], turns), Arc::new(Quiet))
                .finish()
                .await
        } else {
            runner
                .run(context(), request(vec![], turns), Arc::new(Quiet))
                .await
        }
        .unwrap();
        assert_eq!(outcome.result.final_text(), case["final"], "{name}");
        let feedback: Vec<_> = outcome
            .result
            .new_items
            .iter()
            .filter_map(|item| match item {
                RunItem::Message { message } => {
                    message.content.iter().find_map(|content| match content {
                        Content::Text { text } if text.starts_with("[SYSTEM]") => {
                            Some(text.clone())
                        }
                        _ => None,
                    })
                }
                _ => None,
            })
            .collect();
        assert_eq!(json!(feedback), case["feedback"], "{name}");
        let requests = model.requests.lock().unwrap();
        assert_eq!(json!(requests.len()), case["calls"], "{name}");
        assert_eq!(
            json!(
                requests
                    .iter()
                    .map(|request| request.tools.len())
                    .collect::<Vec<_>>()
            ),
            case["tools"],
            "{name}"
        );
        assert_eq!(
            json!(gate.0.load(Ordering::SeqCst)),
            case["gate_calls"],
            "{name}"
        );
        assert_eq!(
            json!(poller.calls.load(Ordering::SeqCst)),
            case["poll_calls"],
            "{name}"
        );
        assert_eq!(
            json!(finalizer.calls.load(Ordering::SeqCst)),
            case["finalizer_calls"],
            "{name}"
        );
    }
}

#[test]
fn completion_confirmation_scalar_codec_defaults_roundtrips_and_applies() {
    for (json, expected) in [
        ("{}", false),
        (r#"{"RequireCompletionConfirmation":null}"#, false),
        (r#"{"RequireCompletionConfirmation":false}"#, false),
        (r#"{"RequireCompletionConfirmation":true}"#, true),
    ] {
        let wire: RunConfigSentinels = serde_json::from_str(json).unwrap();
        let effective = wire.resolve().unwrap();
        assert_eq!(effective.require_completion_confirmation, expected);
        assert_eq!(
            RunConfigSentinels::from_effective(&effective)
                .unwrap()
                .require_completion_confirmation,
            expected
        );
        let mut config = RunnerConfig {
            require_completion_confirmation: !expected,
            ..Default::default()
        };
        apply_go_config(&wire, &mut config, &mut RunPolicy::default()).unwrap();
        assert_eq!(config.require_completion_confirmation, expected);
    }
}

#[tokio::test]
async fn completion_confirmation_cannot_restore_denied_tools_or_exhausted_token_budget() {
    for deny in [false, true] {
        let mut first = response(vec![message(Role::Assistant, "first")], true);
        first.usage.input_tokens = 1;
        let model = Script::new(vec![Ok(first)]);
        let mut agent = AgentConfig::new("agent", ModelBinding::complete("fake", model.clone()));
        agent
            .tools
            .push(TestTool::new("read", true, false, false, false));
        let config = RunnerConfig {
            require_completion_confirmation: true,
            limits: Limits {
                max_tokens: (!deny).then_some(1),
                ..Default::default()
            },
            ..Default::default()
        };
        let runner = Runner::new(agent, config).unwrap();
        let mut req = request(vec![], 1);
        if deny {
            req.policy.tools.denied_tools.insert("read".into());
        }
        let outcome = runner.run(context(), req, Arc::new(Quiet)).await;
        if deny {
            assert_eq!(outcome.unwrap().result.final_text(), "first");
        } else {
            assert_eq!(
                outcome.err().unwrap().error.info.message,
                "run token budget exhausted"
            );
        }
        assert_eq!(model.requests.lock().unwrap().len(), 1);
    }
}

#[derive(Default)]
struct ConfirmationOrdering {
    confirmations: AtomicUsize,
    polls: AtomicUsize,
}
impl RunHooks for ConfirmationOrdering {
    fn observe<'a>(
        &'a self,
        _: &'a Context,
        event: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            if let Observation::CommittedItems { items, .. } = event {
                for item in items {
                    if let RunItem::Message { message } = item {
                        if message.content.iter().any(|content| matches!(content,Content::Text {text} if text.contains("verify your work now"))) {
                            self.confirmations.fetch_add(1,Ordering::SeqCst);
                        }
                    }
                }
            }
            Ok(())
        })
    }
}
impl ImmediateInputPoller for ConfirmationOrdering {
    fn poll<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<ImmediateInputBatch, Error>> {
        Box::pin(async move {
            if self.polls.fetch_add(1, Ordering::SeqCst) == 1 {
                assert_eq!(
                    self.confirmations.load(Ordering::SeqCst),
                    1,
                    "confirmation must be published before the next admission callback"
                );
            }
            Ok(ImmediateInputBatch::default())
        })
    }
}
#[tokio::test]
async fn confirmation_feedback_is_published_before_the_next_input_poll() {
    for streaming in [false, true] {
        let model = Script::new(vec![Ok(answer()), Ok(answer())]);
        let binding = if streaming {
            ModelBinding::streaming("fake", model)
        } else {
            ModelBinding::complete("fake", model)
        };
        let mut agent = AgentConfig::new("agent", binding);
        agent
            .tools
            .push(TestTool::new("read", true, false, false, false));
        let ordering = Arc::new(ConfirmationOrdering::default());
        let runner = Runner::new(
            agent,
            RunnerConfig {
                require_completion_confirmation: true,
                hooks: Some(ordering.clone()),
                immediate_input_poller: Some(ordering.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        if streaming {
            runner
                .stream(context(), request(vec![], 1), Arc::new(Quiet))
                .finish()
                .await
                .unwrap();
        } else {
            runner
                .run(context(), request(vec![], 1), Arc::new(Quiet))
                .await
                .unwrap();
        }
        assert_eq!(ordering.confirmations.load(Ordering::SeqCst), 1);
    }
}

struct VerifierProbe {
    inputs: Mutex<Vec<serde_json::Value>>,
    feedback: String,
    fail: bool,
}
impl FinalAnswerVerifier for VerifierProbe {
    fn verify<'a>(
        &'a self,
        _: &'a Context,
        output: &'a serde_json::Value,
    ) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async move {
            self.inputs.lock().unwrap().push(output.clone());
            if self.fail {
                Err(Error::new(ErrorCategory::Host, "private verifier failure"))
            } else {
                Ok(self.feedback.clone())
            }
        })
    }
}

#[tokio::test]
async fn final_verifier_matches_pinned_sdk_once_after_gate_normal_and_streamed() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/verifier/observations.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let streamed = case["streamed"].as_bool().unwrap();
        let reply = |text: &str| response(vec![message(Role::Assistant, text)], true);
        let mut turns = 1;
        let replies = match name {
            "confirmation" => vec![
                reply("first"),
                reply("confirmed"),
                reply("revised"),
                reply("final"),
            ],
            "gate" => vec![reply("first"), reply("second"), reply("final")],
            "tools" => {
                turns = 3;
                vec![
                    reply("first"),
                    response(vec![call("read")], true),
                    reply("final"),
                ]
            }
            "end_turn" => {
                turns = 3;
                vec![
                    reply("first"),
                    response(vec![message(Role::Assistant, "progress")], false),
                    reply("final"),
                ]
            }
            "object" => vec![reply(r#"{"z":"<&>","a":1}"#)],
            "number" => vec![reply("42")],
            "null" => vec![reply("null")],
            "string" => vec![reply(r#""decoded""#)],
            _ => vec![reply("first"), reply("final")],
        };
        let model = Script::new(replies.into_iter().map(Ok).collect());
        let binding = if streamed {
            ModelBinding::streaming("fake", model.clone())
        } else {
            ModelBinding::complete("fake", model.clone())
        };
        let mut agent = AgentConfig::new("agent", binding);
        if name != "no_tools" {
            agent
                .tools
                .push(TestTool::new("read", true, false, false, false));
        }
        if matches!(name, "object" | "number" | "null" | "string") {
            agent.output_schema = Some(schemars::json_schema!(true));
        }
        let verifier = Arc::new(VerifierProbe {
            inputs: Mutex::new(vec![]),
            feedback: match name {
                "approve" | "object" | "number" | "null" | "string" => "",
                "blank" => " \n",
                _ => "fix this",
            }
            .into(),
            fail: name == "error",
        });
        let gate = Arc::new(ConfirmationGate(AtomicUsize::new(0)));
        let hooks = Arc::new(Hooks::default());
        let mut config = RunnerConfig {
            final_answer_verifier: Some(verifier.clone()),
            require_completion_confirmation: name == "confirmation",
            force_final_summary_turn: name == "summary",
            hooks: Some(hooks.clone()),
            ..Default::default()
        };
        if name == "gate" {
            config.stop_gate = Some(gate.clone());
            config.stop_gate_max_blocks = 1;
        }
        let runner = Runner::new(agent, config).unwrap();
        let outcome = if streamed {
            runner
                .stream(context(), request(vec![], turns), Arc::new(Quiet))
                .finish()
                .await
        } else {
            runner
                .run(context(), request(vec![], turns), Arc::new(Quiet))
                .await
        }
        .unwrap();
        assert_eq!(
            outcome.result.final_output.as_ref().unwrap(),
            &case["final_output"],
            "{name}"
        );
        let feedback: Vec<_> = outcome
            .result
            .new_items
            .iter()
            .filter_map(|item| match item {
                RunItem::Message { message } => {
                    message.content.iter().find_map(|content| match content {
                        Content::Text { text } if text.starts_with("[SYSTEM]") => {
                            Some(text.clone())
                        }
                        _ => None,
                    })
                }
                _ => None,
            })
            .collect();
        assert_eq!(json!(feedback), case["feedback"], "{name}");
        let inputs: Vec<_> = verifier
            .inputs
            .lock()
            .unwrap()
            .iter()
            .map(|value| match value {
                serde_json::Value::Null => String::new(),
                serde_json::Value::String(text) => text.clone(),
                _ => String::from_utf8(adk_codec::snapshots::to_go_json(value).unwrap()).unwrap(),
            })
            .collect();
        assert_eq!(json!(inputs), case["verifier_inputs"], "{name}");
        assert_eq!(
            json!(gate.0.load(Ordering::SeqCst)),
            case["gate_calls"],
            "{name}"
        );
        assert_eq!(
            json!(model.requests.lock().unwrap().len()),
            case["calls"],
            "{name}"
        );
        assert_eq!(
            json!(
                model
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|req| req.tools.len())
                    .collect::<Vec<_>>()
            ),
            case["tools"],
            "{name}"
        );
        assert_eq!(hooks.events.lock().unwrap().iter().filter(|event|matches!(event,Observation::FinalAnswerVerificationFailed {error} if error.message=="private verifier failure")).count(),usize::from(name=="error"));
    }
}

struct CancellingVerifier(Arc<CancellationToken>);
impl FinalAnswerVerifier for CancellingVerifier {
    fn verify<'a>(
        &'a self,
        _: &'a Context,
        _: &'a serde_json::Value,
    ) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async move {
            self.0.cancel();
            Err(Error::new(ErrorCategory::Host, "verification stopped"))
        })
    }
}
#[tokio::test]
async fn verifier_failure_never_swallows_parent_cancellation() {
    let token = Arc::new(CancellationToken::new());
    let ctx = Context {
        cancellation: token.clone(),
        ..context()
    };
    let model = Script::new(vec![Ok(answer())]);
    let mut agent = AgentConfig::new("agent", ModelBinding::complete("fake", model.clone()));
    agent
        .tools
        .push(TestTool::new("read", true, false, false, false));
    let runner = Runner::new(
        agent,
        RunnerConfig {
            final_answer_verifier: Some(Arc::new(CancellingVerifier(token))),
            ..Default::default()
        },
    )
    .unwrap();
    let error = runner
        .run(ctx, request(vec![], 1), Arc::new(Quiet))
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::Cancelled);
    assert_eq!(model.requests.lock().unwrap().len(), 1);
}

struct CriticReadTool {
    definition: ToolDefinition,
    turns: Mutex<Vec<u32>>,
}
impl Tool for CriticReadTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        ctx: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            assert_eq!(ctx.policy.access, AccessMode::ReadOnly);
            self.turns
                .lock()
                .unwrap()
                .push(ctx.policy.max_child_turns.unwrap().get());
            Ok(ToolOutput {
                content: vec![Content::Text { text: "ok".into() }],
                is_error: false,
                should_pause: false,
            })
        })
    }
}

#[tokio::test]
async fn critic_matches_pinned_sdk_verdicts_prompts_and_read_only_turn_limits() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/critic/observations.json")).unwrap();
    assert_eq!(DEFAULT_CRITIC_INSTRUCTIONS, fixture["default_instructions"]);
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut replies = vec![];
        if matches!(name, "read" | "write" | "turn_cap") {
            for i in 0..if name == "turn_cap" { 11 } else { 1 } {
                replies.push(Ok(response(
                    vec![RunItem::ToolCall {
                        call: ToolCall {
                            id: format!("call_{i}"),
                            name: if name == "write" { "write" } else { "read" }.into(),
                            arguments: json!({}),
                        },
                    }],
                    false,
                )));
            }
        }
        replies.push(if name == "error" {
            Err(Error::new(ErrorCategory::Provider, "no more responses"))
        } else {
            Ok(response(
                vec![message(Role::Assistant, case["reply"].as_str().unwrap())],
                true,
            ))
        });
        let model = Script::new(replies);
        let read = Arc::new(CriticReadTool {
            definition: ToolDefinition {
                name: "read".into(),
                description: String::new(),
                input_schema: schemars::json_schema!({}),
                read_only: true,
                requires_approval: false,
            },
            turns: Mutex::new(vec![]),
        });
        let write = TestTool::new("write", false, false, false, false);
        let mut agent =
            AgentConfig::new("critic", ModelBinding::complete("offline", model.clone()));
        agent.instructions = case["instructions"].as_str().unwrap().into();
        agent.tools = vec![read.clone(), write.clone()];
        if case["structured"] == true {
            agent.output_schema = Some(schemars::json_schema!(true));
            agent.output_schema_strict = false;
        }
        let critic =
            CriticVerifier::new(agent, case["task"].as_str().unwrap(), Arc::new(Quiet)).unwrap();
        assert!(critic.durable_key().is_none());
        let result = critic.verify(&context(), &case["candidate"]).await;
        assert_eq!(result.is_err(), case["error"], "{name}");
        if let Ok(feedback) = result {
            assert_eq!(feedback, case["feedback"], "{name}");
        }
        let requests = model.requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            case["requests"].as_u64().unwrap() as usize,
            "{name}"
        );
        assert_eq!(
            json!(requests.iter().map(|r| &r.instructions).collect::<Vec<_>>()),
            case["request_instructions"],
            "{name}"
        );
        assert_eq!(
            json!(
                requests
                    .iter()
                    .map(|r| r.tools.iter().map(|t| &t.name).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            ),
            case["request_tools"],
            "{name}"
        );
        assert_eq!(
            requests[0].input[0],
            message(Role::User, case["prompt"].as_str().unwrap()),
            "{name}"
        );
        assert_eq!(
            json!(*read.turns.lock().unwrap()),
            case["nested_turns"],
            "{name}"
        );
        assert_eq!(
            read.turns.lock().unwrap().len(),
            case["read_calls"].as_u64().unwrap() as usize,
            "{name}"
        );
        assert_eq!(write.calls.load(Ordering::SeqCst), 0, "{name}");
    }
}

#[tokio::test]
async fn critic_refutation_revises_parent_once_and_cancellation_prevents_dispatch() {
    let critic_model = Script::new(vec![Ok(response(
        vec![message(
            Role::Assistant,
            "VERDICT: REJECTED\n1. Missing evidence",
        )],
        true,
    ))]);
    let critic = Arc::new(
        CriticVerifier::new(
            AgentConfig::new(
                "critic",
                ModelBinding::complete("offline", critic_model.clone()),
            ),
            "task",
            Arc::new(Quiet),
        )
        .unwrap(),
    );
    let model = Script::new(vec![
        Ok(answer()),
        Ok(response(vec![message(Role::Assistant, "revised")], true)),
    ]);
    let mut parent = AgentConfig::new("parent", ModelBinding::complete("offline", model.clone()));
    parent
        .tools
        .push(TestTool::new("read", true, false, false, false));
    let runner = Runner::new(
        parent,
        RunnerConfig {
            final_answer_verifier: Some(critic.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let outcome = runner
        .run(
            context(),
            request(vec![message(Role::User, "task")], 1),
            Arc::new(Quiet),
        )
        .await
        .unwrap();
    assert_eq!(outcome.result.final_text(), "revised");
    assert_eq!(critic_model.requests.lock().unwrap().len(), 1);
    assert_eq!(model.requests.lock().unwrap().len(), 2);
    let cancellation = Arc::new(CancellationToken::new());
    cancellation.cancel();
    let ctx = Context {
        cancellation,
        ..context()
    };
    let error = critic.verify(&ctx, &json!("candidate")).await.unwrap_err();
    assert_eq!(error.info.category, ErrorCategory::Cancelled);
    assert_eq!(critic_model.requests.lock().unwrap().len(), 1);
}

struct DynamicInstructions {
    blank: bool,
    calls: Mutex<Vec<String>>,
}
impl InstructionProvider for DynamicInstructions {
    fn instructions<'a>(
        &'a self,
        ctx: InstructionContext<'a>,
    ) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async move {
            let text = format!(
                "dynamic:{}:{}:{}",
                ctx.agent.name, ctx.snapshot.usage.input_tokens, ctx.snapshot.usage.output_tokens
            );
            self.calls.lock().unwrap().push(text.clone());
            Ok(if self.blank { String::new() } else { text })
        })
    }
}

#[tokio::test]
async fn dynamic_instructions_match_pinned_static_precedence_usage_composition_and_handoff() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/dynamic-instructions/observations.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let streamed = case["streamed"].as_bool().unwrap();
        let mut replies = vec![];
        if matches!(name, "progress" | "handoff") {
            let mut first = response(
                vec![if name == "progress" {
                    message(Role::Assistant, "progress")
                } else {
                    RunItem::ToolCall {
                        call: ToolCall {
                            id: "transfer".into(),
                            name: "transfer_to_child".into(),
                            arguments: json!({}),
                        },
                    }
                }],
                false,
            );
            first.usage.input_tokens = 7;
            first.usage.output_tokens = 3;
            replies.push(Ok(first));
        }
        if matches!(name, "retry" | "summary_retry") {
            replies.push(Err(Error::new(ErrorCategory::Provider, "transient")));
        }
        replies.push(Ok(answer()));
        let model = Script::new(replies);
        let binding = if streamed {
            ModelBinding::streaming("offline", model.clone())
        } else {
            ModelBinding::complete("offline", model.clone())
        };
        let provider = Arc::new(DynamicInstructions {
            blank: name == "blank",
            calls: Mutex::new(vec![]),
        });
        let mut agent = AgentConfig::new("agent", binding.clone());
        agent.instructions = "static fallback".into();
        if name != "static" {
            agent.instruction_provider = Some(provider.clone());
        }
        let mut config = RunnerConfig::default();
        if matches!(name, "retry" | "summary_retry") {
            config.retry.max_retries = 1;
            config.retry.initial_delay = Duration::from_millis(1);
        }
        if name == "compose" {
            config.additional_instructions = " additional instructions ".into();
            agent.mcp_servers = vec!["files".into()];
        }
        if name == "handoff" {
            let mut child = AgentConfig::new("child", binding);
            child.instructions = "child fallback".into();
            child.instruction_provider = Some(provider.clone());
            agent.handoffs.push(Handoff {
                on_handoff: None,
                is_enabled: None,
                definition: ToolDefinition {
                    name: "transfer_to_child".into(),
                    description: String::new(),
                    input_schema: schemars::json_schema!({}),
                    read_only: true,
                    requires_approval: false,
                },
                target: Arc::new(child),
                input_filter: HandoffInputFilter::Preserve,
            });
        }
        if name == "summary_retry" {
            config.force_final_summary_turn = true;
        }
        let turns = if name == "summary_retry" { 2 } else { 4 };
        let runner = Runner::new(agent, config).unwrap();
        let outcome = if streamed {
            runner
                .stream(context(), request(vec![], turns), Arc::new(Quiet))
                .finish()
                .await
                .unwrap()
        } else {
            runner
                .run(context(), request(vec![], turns), Arc::new(Quiet))
                .await
                .unwrap()
        };
        assert_eq!(
            outcome.result.final_text(),
            case["final"],
            "{name}/{streamed}"
        );
        assert_eq!(
            json!(*provider.calls.lock().unwrap()),
            case["calls"],
            "{name}/{streamed}"
        );
        assert_eq!(
            json!(
                model
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|r| &r.instructions)
                    .collect::<Vec<_>>()
            ),
            case["instructions"],
            "{name}/{streamed}"
        );
    }
}

struct PendingInstructions {
    entered: tokio::sync::Notify,
    dropped: AtomicUsize,
    fail: bool,
}
impl InstructionProvider for PendingInstructions {
    fn instructions<'a>(
        &'a self,
        _: InstructionContext<'a>,
    ) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async move {
            struct DropProbe<'a>(&'a AtomicUsize);
            impl Drop for DropProbe<'_> {
                fn drop(&mut self) {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
            }
            let _probe = DropProbe(&self.dropped);
            self.entered.notify_one();
            if self.fail {
                return Err(Error::new(ErrorCategory::Host, "instructions unavailable"));
            }
            std::future::pending().await
        })
    }
}
#[tokio::test]
async fn dynamic_instructions_failure_cancellation_and_drop_never_dispatch_a_model() {
    for mode in ["fail", "cancel", "drop"] {
        let provider = Arc::new(PendingInstructions {
            entered: tokio::sync::Notify::new(),
            dropped: AtomicUsize::new(0),
            fail: mode == "fail",
        });
        let model = Script::new(vec![]);
        let mut agent = AgentConfig::new("agent", ModelBinding::complete("offline", model.clone()));
        agent.instruction_provider = Some(provider.clone());
        let runner = Runner::new(agent, RunnerConfig::default()).unwrap();
        let cancellation = Arc::new(CancellationToken::new());
        let ctx = Context {
            cancellation: cancellation.clone(),
            ..context()
        };
        let mut future = Box::pin(runner.run(ctx, request(vec![], 1), Arc::new(Quiet)));
        if mode == "fail" {
            assert_eq!(
                future.await.err().unwrap().error.info.message,
                "instructions unavailable"
            );
        } else {
            tokio::select! {
                biased;
                _ = &mut future => panic!("unexpected completion"),
                _ = provider.entered.notified() => {}
            }
            if mode == "cancel" {
                cancellation.cancel();
                assert_eq!(
                    future.await.err().unwrap().error.info.category,
                    ErrorCategory::Cancelled
                );
            } else {
                drop(future);
            }
        }
        assert_eq!(provider.dropped.load(Ordering::SeqCst), 1);
        assert!(model.requests.lock().unwrap().is_empty());
    }
}

struct StopTool(ToolDefinition);
impl Tool for StopTool {
    fn definition(&self) -> &ToolDefinition {
        &self.0
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            Ok(ToolOutput {
                content: vec![Content::Text {
                    text: format!("{}-output", self.0.name),
                }],
                is_error: false,
                should_pause: self.0.name == "present_plan",
            })
        })
    }
}
struct StopGuard {
    values: Mutex<Vec<serde_json::Value>>,
    trip: bool,
}
impl Guardrail for StopGuard {
    fn name(&self) -> &str {
        "check"
    }
    fn check<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
        input: GuardrailInput<'a>,
    ) -> BoxFuture<'a, Result<Option<GuardrailResult>, Error>> {
        Box::pin(async move {
            let GuardrailInput::Output(output) = input else {
                panic!("wrong guardrail phase")
            };
            self.values.lock().unwrap().push(output.clone());
            Ok(Some(GuardrailResult {
                tripwire_triggered: self.trip,
                ..Default::default()
            }))
        })
    }
}
#[tokio::test]
async fn per_agent_tool_stopping_matches_pinned_sdk_outputs_names_schema_and_guardrails() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/tool-stopping/observations.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let streamed = case["streamed"].as_bool().unwrap();
        let called = match name {
            "pause" => "present_plan",
            "handoff" => "transfer_to_child",
            _ => "read",
        };
        let mut items = vec![call(called)];
        if name == "second_named" {
            items.push(call("other"));
        }
        let model = Script::new(vec![Ok(response(items, true)), Ok(answer())]);
        let mut agent = AgentConfig::new(
            "agent",
            if streamed {
                ModelBinding::streaming("offline", model.clone())
            } else {
                ModelBinding::complete("offline", model.clone())
            },
        );
        agent.tools = [
            if name == "pause" {
                "present_plan"
            } else {
                "read"
            },
            "other",
        ]
        .into_iter()
        .map(|name| {
            Arc::new(StopTool(ToolDefinition {
                name: name.into(),
                description: String::new(),
                input_schema: schemars::json_schema!({}),
                read_only: true,
                requires_approval: false,
            })) as Arc<dyn Tool>
        })
        .collect();
        let guard = Arc::new(StopGuard {
            values: Mutex::new(vec![]),
            trip: name == "guardrail",
        });
        agent.output_guardrails.push(guard.clone());
        agent.stop_at_tools.insert(
            match name {
                "case_miss" => "READ",
                "space_miss" => " read ",
                "second_named" => "other",
                _ => called,
            }
            .into(),
        );
        if matches!(name, "continue" | "stop_all") {
            agent.stop_at_tools.clear();
        }
        if name == "stop_all" {
            agent.tool_use = ToolUseBehavior::StopAfterTool;
        }
        if matches!(
            name,
            "first" | "second_named" | "schema" | "continue" | "steering"
        ) {
            agent.tool_final_output = Some(ToolFinalOutput::FirstTool);
        }
        if name == "schema" {
            agent.output_schema = Some(schemars::json_schema!({"type":"object"}));
        }
        if matches!(name, "object" | "string" | "null" | "false") {
            agent.tool_final_output = Some(ToolFinalOutput::Json(case["output"].clone()));
        }
        if name == "handoff" {
            agent.handoffs.push(Handoff {
                on_handoff: None,
                is_enabled: None,
                definition: ToolDefinition {
                    name: "transfer_to_child".into(),
                    description: String::new(),
                    input_schema: schemars::json_schema!({}),
                    read_only: true,
                    requires_approval: false,
                },
                target: Arc::new(AgentConfig::new("child", agent.model.clone())),
                input_filter: HandoffInputFilter::Preserve,
            });
        }
        let mut config = RunnerConfig::default();
        if name == "steering" {
            config.immediate_input_finalizer = Some(Arc::new(ConfirmationInput {
                at: 1,
                calls: AtomicUsize::new(0),
            }));
        }
        let runner = Runner::new(agent, config).unwrap();
        let result = if streamed {
            runner
                .stream(context(), request(vec![], 3), Arc::new(Quiet))
                .finish()
                .await
        } else {
            runner
                .run(context(), request(vec![], 3), Arc::new(Quiet))
                .await
        };
        assert_eq!(result.is_err(), case["error"], "{name}/{streamed}");
        if let Ok(result) = result {
            assert_eq!(
                json!(result.result.final_output),
                case["output"],
                "{name}/{streamed}"
            );
            assert_eq!(
                result.result.final_text(),
                case["text"],
                "{name}/{streamed}"
            );
            assert_eq!(
                result.result.final_output_is_raw_json,
                matches!(name, "object" | "string" | "null" | "false")
            );
        }
        assert_eq!(
            json!(*guard.values.lock().unwrap()),
            case["guards"],
            "{name}/{streamed}"
        );
        assert_eq!(
            model.requests.lock().unwrap().len(),
            case["requests"].as_u64().unwrap() as usize,
            "{name}/{streamed}"
        );
    }
}

#[test]
fn public_handoff_constructor_matches_pinned_defaults_and_explicit_overrides() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff-constructor/observations.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut target = AgentConfig::new(
            case["name"].as_str().unwrap(),
            ModelBinding::complete("offline", Script::new(vec![])),
        );
        target.handoff_description = case["target_description"].as_str().unwrap().into();
        let target = Arc::new(target);
        let mut handoff = Handoff::new(target.clone());
        if case["overrides"] == true {
            handoff.definition.name = "custom_transfer".into();
            handoff.definition.description = case["description"].as_str().unwrap().into();
        }
        assert_eq!(handoff.definition.name, case["tool_name"], "{case}");
        assert_eq!(
            handoff.definition.description, case["description"],
            "{case}"
        );
        assert_eq!(
            handoff.definition.input_schema.as_value(),
            &case["schema"],
            "{case}"
        );
        assert_eq!(handoff.definition.read_only, case["read_only"], "{case}");
        assert_eq!(
            handoff.definition.requires_approval, case["approval"],
            "{case}"
        );
        assert_eq!(handoff.input_filter, HandoffInputFilter::Preserve);
        assert!(Arc::ptr_eq(&handoff.target, &target));
        assert_eq!(target.handoff_description, case["target_description"]);
    }
}

#[tokio::test]
async fn public_handoff_constructor_transfers_to_the_owned_target() {
    let model = Script::new(vec![
        Ok(response(vec![call("transfer_to_Code_Reviewer")], true)),
        Ok(answer()),
    ]);
    let binding = ModelBinding::complete("offline", model.clone());
    let mut target = AgentConfig::new("Code Reviewer", binding.clone());
    target.instructions = "Review the changes".into();
    target.handoff_description = "reviewer".into();
    let mut parent = AgentConfig::new("parent", binding);
    parent.handoffs.push(Handoff::new(Arc::new(target)));
    let runner = Runner::new(parent, RunnerConfig::default()).unwrap();
    let result = runner
        .run(context(), request(vec![], 3), Arc::new(Quiet))
        .await
        .unwrap();
    assert_eq!(result.result.last_agent.as_deref(), Some("Code Reviewer"));
    assert_eq!(result.result.final_text(), "done");
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].tools[0].name, "transfer_to_Code_Reviewer");
    assert_eq!(requests[0].tools[0].description, "reviewer");
    assert_eq!(requests[1].instructions, "Review the changes");
}

struct CallbackHooks {
    label: &'static str,
    events: Arc<Mutex<Vec<String>>>,
}
impl RunHooks for CallbackHooks {
    fn observe<'a>(
        &'a self,
        _: &'a Context,
        event: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            match event {
                Observation::Handoff { from, to } => {
                    assert_eq!((from.as_str(), to.as_str()), ("router", "expert"));
                    self.events.lock().unwrap().push(self.label.into());
                }
                Observation::HandoffInputValidationFailed { .. } if self.label == "run_hook" => {
                    self.events.lock().unwrap().push("schema_warning".into());
                }
                _ => {}
            }
            Ok(())
        })
    }
}
struct SeedHandoff {
    events: Arc<Mutex<Vec<String>>>,
    inputs: Mutex<Vec<Value>>,
    seeded: AtomicUsize,
}
impl HandoffCallback for SeedHandoff {
    fn on_handoff<'a>(
        &'a self,
        context: HandoffContext<'a>,
        input: &'a Value,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            assert_eq!(context.agent.name, "router");
            assert_eq!(context.target.name, "expert");
            assert_eq!(context.operation.run_id, "followup");
            assert!(
                !context
                    .snapshot
                    .new_items
                    .iter()
                    .any(|item| matches!(item, RunItem::Handoff { .. }))
            );
            self.events.lock().unwrap().push("callback".into());
            self.inputs.lock().unwrap().push(input.clone());
            self.seeded.store(1, Ordering::SeqCst);
        })
    }
}
impl InstructionProvider for SeedHandoff {
    fn instructions<'a>(
        &'a self,
        _: InstructionContext<'a>,
    ) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async move {
            self.events
                .lock()
                .unwrap()
                .push("target_instructions".into());
            Ok(if self.seeded.load(Ordering::SeqCst) == 1 {
                "target seeded by callback"
            } else {
                "target original"
            }
            .into())
        })
    }
}

#[tokio::test]
async fn handoff_callback_matches_pinned_structured_inputs_order_and_siblings() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff-callback/observations.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let events = Arc::new(Mutex::new(vec![]));
        let callback = Arc::new(SeedHandoff {
            events: events.clone(),
            inputs: Mutex::new(vec![]),
            seeded: AtomicUsize::new(0),
        });
        let input =
            serde_json::from_str(case["input_raw"].as_str().unwrap()).unwrap_or(Value::Null);
        let outputs = case["filter_outputs"].as_array().unwrap();
        let calls = outputs
            .iter()
            .map(|output| {
                let id = output["call_id"].as_str().unwrap();
                RunItem::ToolCall {
                    call: ToolCall {
                        id: id.into(),
                        name: match id {
                            "lookup" => "lookup",
                            "other" => "transfer_to_other",
                            _ => "transfer_to_expert",
                        }
                        .into(),
                        arguments: if id == "h1" {
                            input.clone()
                        } else {
                            json!({"sibling": true})
                        },
                    },
                }
            })
            .collect();
        let model = Script::new(vec![Ok(response(calls, true)), Ok(answer())]);
        let binding = if case["mode"] == "streamed" {
            ModelBinding::streaming("offline", model.clone())
        } else {
            ModelBinding::complete("offline", model.clone())
        };
        let mut target = AgentConfig::new("expert", binding.clone());
        target.instruction_provider = Some(callback.clone());
        target.hooks = Some(Arc::new(CallbackHooks {
            label: "wrong_target_hook",
            events: events.clone(),
        }));
        let mut handoff = Handoff::new(Arc::new(target));
        if case["has_callback"] == true {
            handoff.on_handoff = Some(callback.clone());
        }
        let mut source = AgentConfig::new("router", binding.clone());
        source.hooks = Some(Arc::new(CallbackHooks {
            label: "old_agent_hook",
            events: events.clone(),
        }));
        source.handoffs = vec![
            handoff,
            Handoff::new(Arc::new(AgentConfig::new("other", binding))),
        ];
        let config = RunnerConfig {
            hooks: Some(Arc::new(CallbackHooks {
                label: "run_hook",
                events: events.clone(),
            })),
            ..Default::default()
        };
        let runner = Runner::new(source, config).unwrap();
        let result = if case["mode"] == "streamed" {
            runner
                .stream(context(), request(vec![], 3), Arc::new(Quiet))
                .finish()
                .await
        } else {
            runner
                .run(context(), request(vec![], 3), Arc::new(Quiet))
                .await
        }
        .unwrap();
        assert_eq!(result.result.last_agent.as_deref(), Some("expert"));
        assert_eq!(result.result.final_text(), case["final_text"]);
        let has_callback = case["has_callback"] == true;
        let mut expected = vec!["old_agent_hook", "run_hook"];
        if has_callback {
            if !input.is_object() {
                expected.push("schema_warning");
            }
            expected.push("callback");
        }
        expected.push("target_instructions");
        assert_eq!(*events.lock().unwrap(), expected, "{case}");
        assert_eq!(
            callback.inputs.lock().unwrap().len(),
            case["callback_count"].as_u64().unwrap() as usize
        );
        if has_callback {
            assert_eq!(callback.inputs.lock().unwrap()[0], input);
        }
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].instructions,
            if has_callback {
                "target seeded by callback"
            } else {
                "target original"
            }
        );
        for output in outputs {
            let id = output["call_id"].as_str().unwrap();
            if id == "h1" {
                assert!(requests[1].input.iter().any(|item| matches!(item, RunItem::Handoff { call_id, agent } if call_id == id && agent == "expert")));
            } else {
                assert!(requests[1].input.iter().any(|item| matches!(item, RunItem::ToolResult { call_id, output: native } if call_id == id && native.is_error && native.content == vec![Content::Text {text: output["content"].as_str().unwrap().into()}])));
            }
        }
    }
}

struct BlockingHandoff {
    entered: tokio::sync::Notify,
    dropped: AtomicUsize,
}
impl HandoffCallback for BlockingHandoff {
    fn on_handoff<'a>(&'a self, _: HandoffContext<'a>, _: &'a Value) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            struct Probe<'a>(&'a AtomicUsize);
            impl Drop for Probe<'_> {
                fn drop(&mut self) {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
            }
            let _probe = Probe(&self.dropped);
            self.entered.notify_one();
            std::future::pending().await
        })
    }
}
#[tokio::test]
async fn handoff_callback_cancellation_drops_future_before_target_dispatch() {
    let callback = Arc::new(BlockingHandoff {
        entered: tokio::sync::Notify::new(),
        dropped: AtomicUsize::new(0),
    });
    let model = Script::new(vec![Ok(response(vec![call("transfer_to_expert")], true))]);
    let binding = ModelBinding::complete("offline", model.clone());
    let mut handoff = Handoff::new(Arc::new(AgentConfig::new("expert", binding.clone())));
    handoff.on_handoff = Some(callback.clone());
    let mut agent = AgentConfig::new("router", binding);
    agent.handoffs.push(handoff);
    let runner = Runner::new(agent, RunnerConfig::default()).unwrap();
    let token = CancellationToken::new();
    let mut ctx = context();
    ctx.cancellation = Arc::new(token.clone());
    let run = runner.run(ctx, request(vec![], 3), Arc::new(Quiet));
    let cancel = async {
        callback.entered.notified().await;
        token.cancel();
    };
    let (result, ()) = tokio::join!(run, cancel);
    assert_eq!(
        result.err().unwrap().error.info.category,
        ErrorCategory::Cancelled
    );
    assert_eq!(callback.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(model.requests.lock().unwrap().len(), 1);
}

struct EnabledHandoff(Arc<AtomicUsize>);
impl HandoffPredicate for EnabledHandoff {
    fn enabled(&self, context: HandoffContext<'_>) -> bool {
        assert_eq!(context.agent.name, "router");
        assert_eq!(context.target.name, "expert");
        self.0.load(Ordering::SeqCst) != 0
    }
}
struct EnabledModel {
    script: Arc<Script>,
    enabled: Arc<AtomicUsize>,
    after: usize,
}
impl Model for EnabledModel {
    fn provider(&self) -> &str {
        "fake"
    }
    fn complete<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            self.enabled.store(self.after, Ordering::SeqCst);
            self.script.complete(context, request).await
        })
    }
}
impl StreamingModel for EnabledModel {
    fn stream<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            Ok(
                Box::new(Events(Some(self.complete(context, request).await?)))
                    as Box<dyn ModelStream>,
            )
        })
    }
}
#[tokio::test]
async fn handoff_enablement_matches_pinned_exposure_reclassification_and_sibling_selection() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff-enabled/observations.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let enabled = Arc::new(AtomicUsize::new(usize::from(case["initial"] == true)));
        let calls = case["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|output| RunItem::ToolCall {
                call: ToolCall {
                    id: output["id"].as_str().unwrap().into(),
                    name: if output["id"] == "h1" {
                        "transfer_to_expert"
                    } else {
                        "transfer_to_other"
                    }
                    .into(),
                    arguments: json!({}),
                },
            })
            .collect();
        let script = Script::new(vec![Ok(response(calls, true)), Ok(answer())]);
        let model = Arc::new(EnabledModel {
            script: script.clone(),
            enabled: enabled.clone(),
            after: usize::from(case["after_request"] == true),
        });
        let binding = if case["streamed"] == true {
            ModelBinding::streaming("offline", model)
        } else {
            ModelBinding::complete("offline", model)
        };
        let callback = Arc::new(SeedHandoff {
            events: Arc::new(Mutex::new(vec![])),
            inputs: Mutex::new(vec![]),
            seeded: AtomicUsize::new(0),
        });
        let mut handoff = Handoff::new(Arc::new(AgentConfig::new("expert", binding.clone())));
        handoff.on_handoff = Some(callback.clone());
        if case["scenario"] != "default" {
            handoff.is_enabled = Some(Arc::new(EnabledHandoff(enabled)));
        }
        let mut source = AgentConfig::new("router", binding.clone());
        source.handoffs.push(handoff);
        if case["scenario"] == "enabled_sibling" {
            source
                .handoffs
                .push(Handoff::new(Arc::new(AgentConfig::new("other", binding))));
        }
        let runner = Runner::new(source, RunnerConfig::default()).unwrap();
        let result = if case["streamed"] == true {
            runner
                .stream(context(), request(vec![], 3), Arc::new(Quiet))
                .finish()
                .await
        } else {
            runner
                .run(context(), request(vec![], 3), Arc::new(Quiet))
                .await
        }
        .unwrap();
        assert_eq!(
            result.result.last_agent.as_deref(),
            case["last_agent"].as_str(),
            "{case}"
        );
        assert_eq!(
            callback.inputs.lock().unwrap().len(),
            case["callbacks"].as_u64().unwrap() as usize
        );
        let requests = script.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            json!(
                requests[0]
                    .tools
                    .iter()
                    .map(|tool| &tool.name)
                    .collect::<Vec<_>>()
            ),
            case["tools"],
            "{case}"
        );
        let outputs: Vec<_> = requests[1].input.iter().filter_map(|item| match item {
            RunItem::Handoff {call_id, agent} => Some(json!({"id":call_id,"error":false,"content":format!("Handing off to {agent}")})),
            RunItem::ToolResult {call_id, output} => Some(json!({"id":call_id,"error":output.is_error,"content":output.content.iter().filter_map(|content| if let Content::Text {text} = content { Some(text.as_str()) } else { None }).collect::<String>()})),
            _ => None,
        }).collect();
        assert_eq!(json!(outputs), case["outputs"], "{case}");
    }
}
