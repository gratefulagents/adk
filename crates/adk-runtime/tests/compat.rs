use adk_codec::{
    approval::{ApprovalPhase, decode_history},
    config::{RunConfigSentinels, ToolPolicySentinels},
    dto,
};
use adk_core::*;
use adk_runtime::{compat::*, *};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    num::NonZeroU32,
    sync::{Arc, Mutex},
    time::Duration,
};

type Log = Arc<Mutex<Vec<String>>>;
fn context() -> Context {
    Context {
        run_id: "compat".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
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
fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: id.into(),
        arguments: json!({}),
    }
}
fn call_item(id: &str) -> RunItem {
    RunItem::ToolCall { call: call(id) }
}
fn output(text: &str) -> ToolOutput {
    ToolOutput {
        content: vec![Content::Text { text: text.into() }],
        is_error: false,
        should_pause: false,
    }
}
fn result_item(id: &str) -> RunItem {
    RunItem::ToolResult {
        call_id: id.into(),
        output: output(id),
    }
}
fn response(items: Vec<RunItem>) -> ModelResponse {
    ModelResponse {
        raw: None,
        items,
        usage: Usage::default(),
        end_turn: None,
        response_id: None,
        metadata: Default::default(),
    }
}
fn request(turns: u32) -> RunRequest {
    RunRequest {
        input: vec![],
        policy: RunPolicy {
            max_turns: NonZeroU32::new(turns).unwrap(),
            ..Default::default()
        },
    }
}
fn provenance(items: &[RunItem]) -> Vec<Option<dto::AgentRef>> {
    items
        .iter()
        .map(|_| {
            Some(dto::AgentRef {
                name: "agent".into(),
            })
        })
        .collect()
}
struct ModelFake(Mutex<VecDeque<ModelResponse>>);
impl Model for ModelFake {
    fn provider(&self) -> &str {
        "fake"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async {
            Ok(self
                .0
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected model call"))
        })
    }
}
fn agent(replies: Vec<ModelResponse>) -> AgentConfig {
    AgentConfig::new(
        "agent",
        ModelBinding::complete("fake", Arc::new(ModelFake(Mutex::new(replies.into())))),
    )
}
struct ToolFake {
    definition: ToolDefinition,
    log: Log,
}
impl ToolFake {
    fn new(name: &str, approval: bool, log: &Log) -> Arc<Self> {
        Arc::new(Self {
            definition: ToolDefinition {
                name: name.into(),
                description: name.into(),
                input_schema: schemars::json_schema!({"type":"object"}),
                read_only: true,
                requires_approval: approval,
            },
            log: log.clone(),
        })
    }
}
impl Tool for ToolFake {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            self.log.lock().unwrap().push(format!("tool:{}", call.id));
            Ok(output(&format!("raw {}", call.id)))
        })
    }
}
struct HostFake;
impl Host for HostFake {
    fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { panic!("Go helper must intercept host approvals") })
    }
}
struct Gate {
    log: Log,
    decisions: Mutex<VecDeque<Result<GoApprovalDecision, Error>>>,
}
impl GoApprovalGate for Gate {
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        request: &'a ApprovalRequest,
    ) -> BoxFuture<'a, Result<GoApprovalDecision, Error>> {
        Box::pin(async move {
            self.log
                .lock()
                .unwrap()
                .push(format!("gate:{}", request.call.id));
            self.decisions
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected approval")
        })
    }
}
fn approve() -> Result<GoApprovalDecision, Error> {
    Ok(GoApprovalDecision {
        approved: true,
        reason: String::new(),
    })
}
struct Callbacks {
    scope: &'static str,
    log: Log,
    panic: bool,
}
impl Callbacks {
    fn record(&self, event: &str) {
        self.log
            .lock()
            .unwrap()
            .push(format!("{}:{event}", self.scope));
        assert!(!self.panic, "ordinary observer panic");
    }
}
impl GoLifecycleCallbacks for Callbacks {
    fn on_agent_start(&self, _: &Context, _: &str) {
        self.record("start");
    }
    fn on_model_start(&self, _: &Context, _: &str, _: &str, _: u32) {
        self.record("model-start");
    }
    fn on_model_end(&self, _: &Context, _: &str, _: &ModelResponse) {
        self.record("model-end");
    }
    fn on_tool_start(&self, _: &Context, _: &str, _: &ToolCall) {
        self.record("tool-start");
    }
    fn on_tool_end(&self, _: &Context, _: &ToolCall, output: &ToolOutput) {
        assert_eq!(
            output.content,
            vec![Content::Text {
                text: "raw tool".into()
            }]
        );
        self.record("tool-end");
    }
    fn on_agent_end(&self, _: &Context, _: &str, output: &Value) {
        assert_eq!(output, &json!("done"));
        self.record("end");
    }
    fn on_handoff(&self, _: &Context, _: &str, _: &str) {
        self.record("handoff");
    }
}
fn callbacks(scope: &'static str, log: &Log, panic: bool) -> Arc<dyn RunHooks> {
    Arc::new(GoCallbackAdapter::new(Arc::new(Callbacks {
        scope,
        log: log.clone(),
        panic,
    })))
}

#[tokio::test]
async fn lifecycle_order_and_raw_output_survive_observer_panics() {
    for panic in [false, true] {
        let log = Log::default();
        let mut agent = agent(vec![
            response(vec![call_item("tool")]),
            response(vec![message("done")]),
        ]);
        agent
            .tools
            .push(ToolFake::new("tool", false, &Log::default()));
        agent.hooks = Some(callbacks("agent", &log, false));
        let config = RunnerConfig {
            hooks: Some(callbacks("run", &log, panic)),
            ..Default::default()
        };
        let outcome = Runner::new(agent, config)
            .unwrap()
            .run(context(), request(2), Arc::new(HostFake))
            .await
            .unwrap();
        assert_eq!(outcome.result.status, RunStatus::Completed);
        assert_eq!(
            *log.lock().unwrap(),
            [
                "run:start",
                "agent:start",
                "run:model-start",
                "agent:model-start",
                "run:model-end",
                "agent:model-end",
                "run:tool-start",
                "agent:tool-start",
                "run:tool-end",
                "agent:tool-end",
                "run:start",
                "agent:start",
                "run:model-start",
                "agent:model-start",
                "run:model-end",
                "agent:model-end",
                "agent:end",
                "run:end",
            ]
        );
    }
}

struct SecurityHook;
impl RunHooks for SecurityHook {
    fn observe<'a>(&'a self, _: &'a Context, _: Observation) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Err(Error::new(ErrorCategory::Guardrail, "security veto")) })
    }
}
#[tokio::test]
async fn native_security_hook_errors_are_not_swallowed() {
    let log = Log::default();
    let mut agent = agent(vec![response(vec![message("done")])]);
    agent.hooks = Some(Arc::new(SecurityHook));
    let runner = Runner::new(
        agent,
        RunnerConfig {
            hooks: Some(callbacks("run", &log, true)),
            ..Default::default()
        },
    )
    .unwrap();
    let error = runner
        .run(context(), request(1), Arc::new(HostFake))
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::Guardrail);
    assert_eq!(error.error.info.message, "security veto");
}

#[tokio::test]
async fn handoff_maps_to_typed_callback() {
    let log = Log::default();
    callbacks("run", &log, false)
        .observe(
            &context(),
            Observation::Handoff {
                from: "a".into(),
                to: "b".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(*log.lock().unwrap(), ["run:handoff"]);
}

#[test]
fn scalar_config_maps_sentinels_without_broadening_authorization() {
    let mut config = RunnerConfig::default();
    let mut policy = RunPolicy::default();
    policy.tools.denied_tools.insert("secret".into());
    policy.tools.timeout = Some(Duration::from_secs(2));
    let effective =
        apply_go_config(&RunConfigSentinels::default(), &mut config, &mut policy).unwrap();
    assert_eq!(policy.max_turns.get(), 100);
    assert_eq!(config.output.max_bytes, Some(16384));
    assert!(config.output.untrusted);
    assert_eq!(config.model_idle_timeout, Some(Duration::from_secs(300)));
    assert_eq!(policy.tools.timeout, Some(Duration::from_secs(2)));
    assert_eq!(effective.sub_agent_max_turns.get(), 50);
    assert_eq!(effective.stop_gate_max_blocks, 8);
    assert_eq!(effective.consecutive_tool_error_limit, Some(3));
    let wire = RunConfigSentinels {
        max_turns: 1,
        max_tool_output_bytes: -1,
        model_call_timeout: -1,
        untrusted_tool_outputs: Some(false),
        tool_policy: Some(ToolPolicySentinels {
            approval_required: false,
            default_timeout: 7,
        }),
        ..Default::default()
    };
    apply_go_config(&wire, &mut config, &mut policy).unwrap();
    assert_eq!(policy.max_turns.get(), 1);
    assert_eq!(config.output.max_bytes, None);
    assert!(!config.output.untrusted);
    assert_eq!(config.model_idle_timeout, None);
    assert_eq!(policy.tools.timeout, Some(Duration::from_secs(7)));
    assert_eq!(policy.tools.approval, ApprovalPolicy::RequiredByTool);
    assert!(policy.tools.denied_tools.contains("secret"));
    assert_eq!(policy.tools.access, AccessMode::ReadOnly);
}

#[test]
fn invalid_config_does_not_partially_apply() {
    let mut config = RunnerConfig::default();
    let mut policy = RunPolicy::default();
    let wire = RunConfigSentinels {
        max_turns: i64::MAX,
        ..Default::default()
    };
    assert!(apply_go_config(&wire, &mut config, &mut policy).is_err());
    assert_eq!(policy, RunPolicy::default());
    assert_eq!(
        config.output.max_bytes,
        RunnerConfig::default().output.max_bytes
    );
}

async fn marker(
    journal: &ApprovalJournal,
    id: &str,
    decision: ApprovalDecision,
    n: usize,
    h: usize,
    reason: Option<&str>,
) {
    journal
        .observe(
            &context(),
            Observation::ApprovalMarker {
                agent: Some("agent".into()),
                call: call(id),
                decision,
                new_items_before: n,
                history_before: h,
                reason: reason.map(str::to_owned),
            },
        )
        .await
        .unwrap();
}
#[tokio::test]
async fn journal_orders_future_boundaries_and_preserves_phases_reasons_and_agents() {
    let journal = ApprovalJournal::default();
    marker(
        &journal,
        "b",
        ApprovalDecision::Deny,
        3,
        4,
        Some("not allowed"),
    )
    .await;
    marker(
        &journal,
        "a",
        ApprovalDecision::Defer,
        2,
        3,
        Some("confirm"),
    )
    .await;
    marker(&journal, "a", ApprovalDecision::Approve, 2, 3, None).await;
    assert!(journal.encode_new_items(&[], &[]).is_err());
    let items = vec![
        call_item("a"),
        call_item("b"),
        result_item("a"),
        result_item("b"),
    ];
    assert!(journal.encode_new_items(&items, &[]).is_err());
    let encoded = journal
        .encode_new_items(&items, &provenance(&items))
        .unwrap();
    assert_eq!(
        encoded.phases,
        [
            ApprovalPhase::Pending,
            ApprovalPhase::Approved,
            ApprovalPhase::Denied
        ]
    );
    assert_eq!(
        encoded.reasons,
        [Some("confirm".into()), None, Some("not allowed".into())]
    );
    let decoded = decode_history(&encoded.items, &encoded.phases).unwrap();
    assert_eq!(decoded.items, items);
    assert_eq!(
        decoded
            .markers
            .iter()
            .map(|m| m.before_item)
            .collect::<Vec<_>>(),
        [2, 2, 3]
    );
    assert!(
        decoded
            .markers
            .iter()
            .all(|m| m.marker.agent.as_ref().unwrap().name == "agent")
    );
    let mut history = vec![message("input")];
    history.extend(items);
    let encoded = journal
        .encode_history(&history, &provenance(&history))
        .unwrap();
    let decoded = decode_history(&encoded.items, &encoded.phases).unwrap();
    assert_eq!(
        decoded
            .markers
            .iter()
            .map(|m| m.before_item)
            .collect::<Vec<_>>(),
        [3, 3, 4]
    );
}

#[tokio::test]
async fn journal_rebases_surviving_calls_and_prunes_removed_history_only() {
    let journal = ApprovalJournal::default();
    let before = vec![
        call_item("old"),
        result_item("old"),
        message("middle"),
        call_item("recent"),
        result_item("recent"),
    ];
    marker(&journal, "old", ApprovalDecision::Approve, 1, 1, None).await;
    marker(
        &journal,
        "recent",
        ApprovalDecision::Defer,
        4,
        4,
        Some("confirm"),
    )
    .await;
    marker(&journal, "recent", ApprovalDecision::Deny, 4, 4, Some("no")).await;
    let after = vec![
        message("summary"),
        call_item("recent"),
        result_item("recent"),
    ];
    journal
        .observe(
            &context(),
            Observation::HistoryReplaced {
                before: before.clone(),
                after: after.clone(),
            },
        )
        .await
        .unwrap();
    let encoded = journal.encode_history(&after, &provenance(&after)).unwrap();
    let decoded = decode_history(&encoded.items, &encoded.phases).unwrap();
    assert_eq!(
        encoded.phases,
        [ApprovalPhase::Pending, ApprovalPhase::Denied]
    );
    assert_eq!(encoded.reasons, [Some("confirm".into()), Some("no".into())]);
    assert_eq!(
        decoded
            .markers
            .iter()
            .map(|m| m.before_item)
            .collect::<Vec<_>>(),
        [2, 2]
    );
    assert_eq!(
        journal
            .encode_new_items(&before, &provenance(&before))
            .unwrap()
            .phases
            .len(),
        3
    );
}

#[tokio::test]
async fn journal_refuses_ambiguous_compaction_instead_of_encoding_stale_anchors() {
    let journal = ApprovalJournal::default();
    marker(&journal, "a", ApprovalDecision::Approve, 1, 1, None).await;
    let before = vec![call_item("a"), result_item("a")];
    let after = vec![call_item("a"), call_item("a"), result_item("a")];
    journal
        .observe(
            &context(),
            Observation::HistoryReplaced {
                before: before.clone(),
                after: after.clone(),
            },
        )
        .await
        .unwrap();
    assert!(journal.encode_history(&after, &provenance(&after)).is_err());
    assert!(
        journal
            .encode_new_items(&before, &provenance(&before))
            .is_ok()
    );
}

fn gate_runner(log: &Log, hooks: Option<Arc<dyn RunHooks>>) -> Runner {
    let mut agent = agent(vec![
        response(vec![
            call_item("first"),
            call_item("free"),
            call_item("second"),
        ]),
        response(vec![message("done")]),
    ]);
    agent.tools = vec![
        ToolFake::new("first", true, log),
        ToolFake::new("free", false, log),
        ToolFake::new("second", true, log),
    ];
    Runner::new(
        agent,
        RunnerConfig {
            hooks,
            ..Default::default()
        },
    )
    .unwrap()
}
#[tokio::test]
async fn go_gate_errors_preserve_eligible_and_prior_approved_effects_in_order() {
    for fail_at in [0, 1] {
        let log = Log::default();
        let mut decisions = VecDeque::new();
        if fail_at == 1 {
            decisions.push_back(approve());
        }
        decisions.push_back(Err(Error::new(ErrorCategory::Host, "gate offline")));
        let gate = Gate {
            log: log.clone(),
            decisions: Mutex::new(decisions),
        };
        let error = run_go_chat(
            &gate_runner(&log, None),
            context(),
            request(1),
            Arc::new(HostFake),
            &gate,
            None,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.error.info.message, "gate offline");
        let partial = error.partial.unwrap();
        let results: Vec<_> = partial
            .new_items
            .iter()
            .filter_map(|item| {
                if let RunItem::ToolResult { call_id, .. } = item {
                    Some(call_id.as_str())
                } else {
                    None
                }
            })
            .collect();
        if fail_at == 0 {
            assert_eq!(*log.lock().unwrap(), ["tool:free", "gate:first"]);
            assert_eq!(results, ["free"]);
        } else {
            assert_eq!(
                *log.lock().unwrap(),
                ["tool:free", "gate:first", "tool:first", "gate:second"]
            );
            assert_eq!(results, ["free", "first"]);
        }
        assert_eq!(partial.status, RunStatus::Incomplete);
    }
}

#[tokio::test]
async fn go_helper_resets_one_turn_budget_and_journals_denial_reason() {
    let log = Log::default();
    let journal = Arc::new(ApprovalJournal::default());
    let gate = Gate {
        log: log.clone(),
        decisions: Mutex::new(
            vec![
                approve(),
                Ok(GoApprovalDecision {
                    approved: false,
                    reason: "custom refusal".into(),
                }),
            ]
            .into(),
        ),
    };
    let outcome = run_go_chat(
        &gate_runner(&log, Some(journal.clone())),
        context(),
        request(1),
        Arc::new(HostFake),
        &gate,
        Some(0),
    )
    .await
    .unwrap();
    assert_eq!(outcome.result.status, RunStatus::Completed);
    assert_eq!(outcome.result.final_output, Some(json!("done")));
    assert_eq!(
        *log.lock().unwrap(),
        ["tool:free", "gate:first", "tool:first", "gate:second"]
    );
    let denied = outcome
        .result
        .new_items
        .iter()
        .find_map(|item| match item {
            RunItem::ToolResult { call_id, output } if call_id == "second" => Some(output),
            _ => None,
        })
        .unwrap();
    assert!(denied.is_error);
    assert!(
        denied
            .content
            .iter()
            .any(|c| matches!(c, Content::Text { text } if text.contains("custom refusal")))
    );
    let encoded = journal
        .encode_new_items(
            &outcome.result.new_items,
            &provenance(&outcome.result.new_items),
        )
        .unwrap();
    assert_eq!(
        encoded.phases,
        [
            ApprovalPhase::Pending,
            ApprovalPhase::Pending,
            ApprovalPhase::Approved,
            ApprovalPhase::Denied
        ]
    );
    assert_eq!(
        encoded.reasons.last().unwrap().as_deref(),
        Some("custom refusal")
    );
    let decoded = decode_history(&encoded.items, &encoded.phases).unwrap();
    assert_eq!(
        decoded
            .markers
            .iter()
            .map(|m| m.before_item)
            .collect::<Vec<_>>(),
        [3, 4, 4, 5]
    );
}

#[tokio::test]
async fn resume_limit_returns_diagnostic_partial_without_executing_next_gate() {
    let log = Log::default();
    let mut agent = agent(vec![
        response(vec![call_item("a")]),
        response(vec![call_item("b")]),
    ]);
    agent.tools = vec![
        ToolFake::new("a", true, &log),
        ToolFake::new("b", true, &log),
    ];
    let runner = Runner::new(agent, RunnerConfig::default()).unwrap();
    let gate = Gate {
        log: log.clone(),
        decisions: Mutex::new(vec![approve()].into()),
    };
    let error = run_go_chat(
        &runner,
        context(),
        request(1),
        Arc::new(HostFake),
        &gate,
        Some(1),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::MaxTurns);
    assert!(error.error.info.message.contains("resume limit"));
    let partial = error.partial.unwrap();
    assert_eq!(partial.status, RunStatus::Incomplete);
    assert_eq!(partial.pending_approvals[0].call.id, "b");
    assert_eq!(*log.lock().unwrap(), ["gate:a", "tool:a"]);
}

#[tokio::test]
async fn default_and_zero_resume_limits_allow_exactly_twelve_resumes() {
    for limit in [None, Some(0)] {
        let log = Log::default();
        let ids: Vec<_> = (0..13).map(|index| format!("call-{index}")).collect();
        let mut agent = agent(ids.iter().map(|id| response(vec![call_item(id)])).collect());
        agent.tools = ids
            .iter()
            .map(|id| ToolFake::new(id, true, &log) as Arc<dyn Tool>)
            .collect();
        let runner = Runner::new(agent, RunnerConfig::default()).unwrap();
        let gate = Gate {
            log: log.clone(),
            decisions: Mutex::new((0..12).map(|_| approve()).collect()),
        };
        let error = run_go_chat(
            &runner,
            context(),
            request(1),
            Arc::new(HostFake),
            &gate,
            limit,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.error.info.category, ErrorCategory::MaxTurns);
        let partial = error.partial.unwrap();
        assert_eq!(partial.pending_approvals[0].call.id, "call-12");
        assert_eq!(log.lock().unwrap().len(), 24);
        assert_eq!(log.lock().unwrap().last().unwrap(), "tool:call-11");
    }
}

struct StreamingFake {
    fail_after_complete: bool,
}
struct StreamFake {
    events: VecDeque<Result<ModelEvent, Error>>,
}
impl ModelStream for StreamFake {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async { self.events.pop_front().transpose() })
    }
}
impl Model for StreamingFake {
    fn provider(&self) -> &str {
        "stream"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async { panic!("expected stream") })
    }
}
impl StreamingModel for StreamingFake {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            let mut events = VecDeque::from([Ok(ModelEvent::Complete {
                response: response(vec![message("done")]),
            })]);
            if self.fail_after_complete {
                events.push_back(Err(Error::new(
                    ErrorCategory::Provider,
                    "late stream error",
                )));
            }
            Ok(Box::new(StreamFake { events }) as Box<dyn ModelStream>)
        })
    }
}
#[tokio::test]
async fn model_end_callback_requires_clean_stream_eof() {
    for fail_after_complete in [false, true] {
        let log = Log::default();
        let agent = AgentConfig::new(
            "agent",
            ModelBinding::streaming(
                "stream",
                Arc::new(StreamingFake {
                    fail_after_complete,
                }),
            ),
        );
        let runner = Runner::new(
            agent,
            RunnerConfig {
                hooks: Some(callbacks("run", &log, false)),
                ..Default::default()
            },
        )
        .unwrap();
        let stream = runner.stream(context(), request(1), Arc::new(HostFake));
        let outcome = stream.finish().await;
        assert_eq!(outcome.is_err(), fail_after_complete);
        assert_eq!(
            log.lock().unwrap().iter().any(|e| e == "run:model-end"),
            !fail_after_complete
        );
    }
}
