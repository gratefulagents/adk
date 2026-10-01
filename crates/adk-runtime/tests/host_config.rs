use adk_core::*;
use adk_runtime::{compaction::LocalCompactionPolicy, subagent::*, *};
use serde_json::json;
use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroU32,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

fn context(id: &str) -> Context {
    Context {
        run_id: id.into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn message(text: &str) -> RunItem {
    RunItem::Message {
        message: Message {
            role: Role::User,
            content: vec![Content::Text { text: text.into() }],
        },
    }
}
fn text(item: &RunItem) -> String {
    match item {
        RunItem::Message { message } => message
            .content
            .iter()
            .filter_map(|c| match c {
                Content::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
fn request(input: Vec<RunItem>) -> RunRequest {
    RunRequest {
        input,
        input_provenance: vec![],
        policy: RunPolicy::default(),
    }
}
fn response(end: bool) -> ModelResponse {
    ModelResponse {
        items: vec![RunItem::Message {
            message: Message {
                role: Role::Assistant,
                content: vec![Content::Text {
                    text: "answer".into(),
                }],
            },
        }],
        end_turn: Some(end),
        usage: Usage {
            context_tokens: Some(100),
            ..Default::default()
        },
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
        Box::pin(async { Ok(ApprovalDecision::Approve) })
    }
}
#[derive(Default)]
struct ModelImpl {
    responses: Mutex<VecDeque<ModelResponse>>,
    requests: Mutex<Vec<ModelRequest>>,
    child_calls: Mutex<HashMap<String, usize>>,
    endless: bool,
}
impl Model for ModelImpl {
    fn provider(&self) -> &str {
        "test"
    }
    fn complete<'a>(
        &'a self,
        ctx: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            *self
                .child_calls
                .lock()
                .unwrap()
                .entry(ctx.run_id.clone())
                .or_default() += 1;
            self.requests.lock().unwrap().push(request);
            tokio::task::yield_now().await;
            Ok(if self.endless {
                response(false)
            } else {
                self.responses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| response(true))
            })
        })
    }
}
fn runner(model: Arc<ModelImpl>, config: RunnerConfig) -> Runner {
    Runner::new(
        AgentConfig::new("agent", ModelBinding::complete("test", model)),
        config,
    )
    .unwrap()
}
struct Carry {
    value: &'static str,
    key: Option<&'static str>,
    fail: bool,
    calls: AtomicUsize,
}
impl CompactionCarryForward for Carry {
    fn context<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err(Error::new(ErrorCategory::Host, "carry callback failed"))
            } else {
                Ok(self.value.into())
            }
        })
    }
    fn durable_key(&self) -> Option<&str> {
        self.key
    }
}
fn carry(value: &'static str, key: Option<&'static str>) -> Arc<Carry> {
    Arc::new(Carry {
        value,
        key,
        fail: false,
        calls: AtomicUsize::new(0),
    })
}
struct EchoCompactor;
impl Compactor for EchoCompactor {
    fn compact<'a>(
        &'a self,
        _: &'a Context,
        request: CompactionRequest,
    ) -> BoxFuture<'a, Result<CompactedHistory, Error>> {
        Box::pin(async move {
            Ok(CompactedHistory {
                history: request.history,
                history_provenance: request.history_provenance,
                context_tokens: 1,
                usage: Usage::default(),
                cost: 0.0,
            })
        })
    }
}
fn custom_config(callback: Option<Arc<dyn CompactionCarryForward>>) -> RunnerConfig {
    RunnerConfig {
        working_state_context: "  static state  ".into(),
        compaction_carry_forward: callback,
        compaction: Some(CompactionConfig {
            trigger_tokens: 50,
            target_tokens: 10,
            compactor: Arc::new(EchoCompactor),
        }),
        local_compaction: LocalCompactionPolicy {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    }
}
const PREFIX: &str = "[COMPACTION CARRY-FORWARD]\nThis live runtime state was injected after context compaction. Treat it as current and higher priority than older compacted history.\n\n";

#[tokio::test]
async fn carry_forward_only_after_compaction_dynamic_precedence_fallback_and_deduplication() {
    for (dynamic, expected) in [("  live state  ", "live state"), (" \n ", "static state")] {
        let callback = carry(dynamic, None);
        let model = Arc::new(ModelImpl {
            responses: Mutex::new(vec![response(false), response(false), response(true)].into()),
            ..Default::default()
        });
        let mut config = custom_config(Some(callback.clone()));
        config.transient_context = vec![message("transient")];
        let outcome = runner(model.clone(), config)
            .run(
                context("carry"),
                request(vec![message("original")]),
                Arc::new(HostImpl),
            )
            .await
            .unwrap();
        assert_eq!(callback.calls.load(Ordering::SeqCst), 2);
        let requests = model.requests.lock().unwrap();
        assert!(
            !requests[0]
                .input
                .iter()
                .any(|item| text(item).contains("state"))
        );
        for req in requests.iter().skip(1) {
            let indices: Vec<_> = req
                .input
                .iter()
                .enumerate()
                .filter(|(_, item)| text(item).starts_with("[COMPACTION CARRY-FORWARD]"))
                .collect();
            assert_eq!(indices.len(), 1);
            let (index, item) = indices[0];
            assert_eq!(text(item), format!("{PREFIX}{expected}"));
            assert_eq!(req.input_provenance[index], ItemProvenance::Unattributed);
            assert_eq!(text(req.input.last().unwrap()), "transient");
        }
        assert_eq!(
            outcome.result.history.len(),
            outcome.result.history_provenance.len()
        );
        assert!(
            !outcome
                .result
                .new_items
                .iter()
                .any(|item| text(item).starts_with("[COMPACTION"))
        );
    }
}

#[tokio::test]
async fn local_compaction_injects_static_context_and_keeps_provenance_aligned() {
    let model = Arc::new(ModelImpl::default());
    let config = RunnerConfig {
        working_state_context: " local state ".into(),
        local_compaction: LocalCompactionPolicy {
            trigger_tokens: 1000,
            target_tokens: 500,
            preserve_recent_items: 1,
            preserve_initial_user_messages: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut input = vec![
        message("original task"),
        message("old ".repeat(5000).as_str()),
        message("[COMPACTION CARRY-FORWARD]\nstale"),
        message("recent"),
    ];
    if let RunItem::Message { message } = &mut input[1] {
        message.role = Role::Assistant;
    }
    let mut req = request(input);
    req.input_provenance = vec![ItemProvenance::Unattributed; 4];
    let outcome = runner(model.clone(), config)
        .run(context("local"), req, Arc::new(HostImpl))
        .await
        .unwrap();
    let requests = model.requests.lock().unwrap();
    let items = &requests[0].input;
    assert_eq!(text(items.last().unwrap()), format!("{PREFIX}local state"));
    assert_eq!(
        items
            .iter()
            .filter(|item| text(item).starts_with("[COMPACTION CARRY-FORWARD]"))
            .count(),
        1
    );
    assert_eq!(items.len(), requests[0].input_provenance.len());
    assert_eq!(
        requests[0].input_provenance.last(),
        Some(&ItemProvenance::Unattributed)
    );
    assert_eq!(
        outcome.result.history.len(),
        outcome.result.history_provenance.len()
    );
}

struct BlockCarry;
impl Guardrail for BlockCarry {
    fn name(&self) -> &str {
        "block-carry"
    }
    fn check<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
        input: GuardrailInput<'a>,
    ) -> BoxFuture<'a, Result<Option<GuardrailResult>, Error>> {
        Box::pin(async move {
            Ok(Some(GuardrailResult {
                tripwire_triggered: matches!(input, GuardrailInput::Input(items) if items.iter().any(|item| text(item).starts_with("[COMPACTION CARRY-FORWARD]"))),
                ..Default::default()
            }))
        })
    }
}
#[tokio::test]
async fn carry_callback_errors_and_input_guardrail_tripwires_fail_closed() {
    for callback_failure in [false, true] {
        let model = Arc::new(ModelImpl {
            responses: Mutex::new(vec![response(false)].into()),
            ..Default::default()
        });
        let callback = Arc::new(Carry {
            value: "live",
            key: None,
            fail: callback_failure,
            calls: AtomicUsize::new(0),
        });
        let mut agent = AgentConfig::new("agent", ModelBinding::complete("test", model.clone()));
        agent.input_guardrails.push(Arc::new(BlockCarry));
        let error = Runner::new(agent, custom_config(Some(callback)))
            .unwrap()
            .run(
                context("blocked"),
                request(vec![message("task")]),
                Arc::new(HostImpl),
            )
            .await
            .err()
            .unwrap();
        assert_eq!(model.requests.lock().unwrap().len(), 1);
        let description = format!("{error:?}");
        assert!(
            description.contains(if callback_failure {
                "carry callback failed"
            } else {
                "block-carry"
            }),
            "{description}"
        );
    }
}

#[tokio::test]
async fn child_turn_caps_are_per_invocation_for_both_adapters_and_clamped_by_scheduler() {
    for agent_tool in [false, true] {
        let model = Arc::new(ModelImpl {
            endless: true,
            ..Default::default()
        });
        let executor = RunnerChildExecutor::new(
            [(
                "worker".into(),
                runner(model.clone(), RunnerConfig::default()),
            )]
            .into_iter()
            .collect(),
            Arc::new(HostImpl),
        );
        let owner = Scheduler::new(
            context("scheduler"),
            SchedulerConfig {
                max_concurrency: 2,
                max_turns: NonZeroU32::new(4).unwrap(),
                agents: [("worker".into(), SecurityBaseline::default())]
                    .into_iter()
                    .collect(),
                ..Default::default()
            },
            Arc::new(executor),
            None,
        )
        .unwrap();
        let session = Arc::new(SubagentSession::new(owner.handle()));
        assert_eq!(session.scheduler.max_concurrency(), 2);
        assert_eq!(session.scheduler.clone().max_concurrency(), 2);
        let make = |id: &str, config_limit: Option<u32>, request_limit: Option<u32>| {
            let tool_name = if agent_tool {
                "worker_tool"
            } else {
                "subagent"
            };
            let mut call = response(false);
            call.items = vec![RunItem::ToolCall {
                call: ToolCall {
                    id: id.into(),
                    name: tool_name.into(),
                    arguments: json!({"message":id}),
                },
            }];
            let parent_model = Arc::new(ModelImpl {
                responses: Mutex::new(vec![call, response(true)].into()),
                ..Default::default()
            });
            let mut agent =
                AgentConfig::new("parent", ModelBinding::complete("test", parent_model));
            agent.tools = if agent_tool {
                vec![Arc::new(AgentAsTool::new(
                    tool_name,
                    "worker",
                    "worker",
                    session.clone(),
                )) as Arc<dyn Tool>]
            } else {
                build_subagent_task_tools(session.clone(), "worker")
            };
            let parent = Runner::new(
                agent,
                RunnerConfig {
                    subagent_max_turns: config_limit.and_then(NonZeroU32::new),
                    subagents: Some(session.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
            let mut req = request(vec![message(id)]);
            req.policy.tools.max_child_turns = request_limit.and_then(NonZeroU32::new);
            let ctx = context(id);
            async move { parent.run(ctx, req, Arc::new(HostImpl)).await.unwrap() }
        };
        tokio::join!(
            make("low", Some(3), Some(1)),
            make("high", Some(8), None),
            make("request", None, Some(2)),
            make("default", None, None)
        );
        let mut counts: Vec<_> = model
            .child_calls
            .lock()
            .unwrap()
            .values()
            .copied()
            .collect();
        counts.sort();
        assert_eq!(counts, vec![1, 2, 4, 4], "adapter agent_tool={agent_tool}");
        assert_eq!(owner.handle().list().len(), 4);
        owner.shutdown().await.unwrap();
    }
}

#[derive(Default)]
struct Store(Mutex<Option<RunnerCheckpoint>>);
impl CheckpointStore for Store {
    fn persist<'a>(
        &'a self,
        _: &'a Context,
        checkpoint: &'a RunnerCheckpoint,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            *self.0.lock().unwrap() = Some(checkpoint.clone());
            Ok(())
        })
    }
}
#[tokio::test]
async fn durable_host_fingerprint_keys_callbacks_and_rejects_configuration_changes() {
    let store = Arc::new(Store::default());
    let config = RunnerConfig {
        working_state_context: "state".into(),
        subagent_max_turns: NonZeroU32::new(3),
        compaction_carry_forward: Some(carry("live", Some("v1"))),
        ..Default::default()
    };
    runner(Arc::new(ModelImpl::default()), config.clone())
        .run_durable(
            context("durable"),
            request(vec![message("task")]),
            Arc::new(HostImpl),
            DurableRun::new(store.clone()),
        )
        .await
        .unwrap();
    let checkpoint = store.0.lock().unwrap().clone().unwrap();
    for change in 0..5 {
        let mut changed = config.clone();
        match change {
            0 => changed.working_state_context = "changed".into(),
            1 => changed.subagent_max_turns = NonZeroU32::new(2),
            2 => changed.compaction_carry_forward = Some(carry("live", Some("v2"))),
            3 => changed.compaction_carry_forward = Some(carry("live", None)),
            _ => changed.compaction_carry_forward = Some(carry("live", Some(""))),
        }
        let model = Arc::new(ModelImpl::default());
        let mut durable = DurableRun::new(store.clone());
        durable.resume = Some(checkpoint.clone());
        assert!(
            runner(model.clone(), changed)
                .run_durable(
                    context("durable"),
                    request(vec![]),
                    Arc::new(HostImpl),
                    durable
                )
                .await
                .is_err()
        );
        assert!(model.requests.lock().unwrap().is_empty());
    }
    let mut durable = DurableRun::new(store);
    durable.resume = Some(checkpoint);
    runner(Arc::new(ModelImpl::default()), config)
        .run_durable(
            context("durable"),
            request(vec![]),
            Arc::new(HostImpl),
            durable,
        )
        .await
        .unwrap();
}

#[test]
fn child_policy_serialization_and_security_narrowing() {
    let old = serde_json::to_value(ToolPolicy::default()).unwrap();
    assert!(old.get("max_child_turns").is_none());
    assert_eq!(
        serde_json::from_value::<ToolPolicy>(old)
            .unwrap()
            .max_child_turns,
        None
    );
    let limited = SecurityBaseline {
        tools: ToolPolicy {
            max_child_turns: NonZeroU32::new(2),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(
        limited
            .narrow(&SecurityBaseline::default())
            .tools
            .max_child_turns,
        NonZeroU32::new(2)
    );
    assert!(!limited.allows_resume_under(&SecurityBaseline::default()));
    assert!(SecurityBaseline::default().allows_resume_under(&limited));
}

#[test]
fn go_config_applies_the_resolved_child_turn_limit() {
    let mut config = RunnerConfig::default();
    let mut policy = RunPolicy::default();
    for (raw, expected) in [(0, 50), (7, 7)] {
        compat::apply_go_config(
            &adk_codec::config::RunConfigSentinels {
                sub_agent_max_turns: raw,
                ..Default::default()
            },
            &mut config,
            &mut policy,
        )
        .unwrap();
        assert_eq!(config.subagent_max_turns, NonZeroU32::new(expected));
    }
}

#[tokio::test]
async fn no_compaction_does_not_consult_live_state() {
    let callback = Arc::new(Carry {
        value: "unused",
        key: None,
        fail: true,
        calls: AtomicUsize::new(0),
    });
    let model = Arc::new(ModelImpl::default());
    runner(
        model.clone(),
        RunnerConfig {
            working_state_context: "unused static".into(),
            compaction_carry_forward: Some(callback.clone()),
            ..Default::default()
        },
    )
    .run(
        context("no-compaction"),
        request(vec![message("task")]),
        Arc::new(HostImpl),
    )
    .await
    .unwrap();
    assert_eq!(callback.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        model.requests.lock().unwrap()[0].input,
        vec![message("task")]
    );
}

struct ApprovedTool(ToolDefinition);
impl Tool for ApprovedTool {
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
                content: vec![Content::Text {
                    text: "tool result".into(),
                }],
                is_error: false,
                should_pause: false,
            })
        })
    }
}
#[tokio::test]
async fn replacing_carry_forward_preserves_approval_markers_and_item_sources() {
    let call = ToolCall {
        id: "approved".into(),
        name: "approved".into(),
        arguments: json!({}),
    };
    let mut first = response(false);
    first.items = vec![RunItem::ToolCall { call: call.clone() }];
    let model = Arc::new(ModelImpl {
        responses: Mutex::new(vec![first, response(true)].into()),
        ..Default::default()
    });
    let journal = Arc::new(compat::ApprovalJournal::default());
    let mut config = custom_config(None);
    config.hooks = Some(journal.clone());
    let mut agent = AgentConfig::new("agent", ModelBinding::complete("test", model.clone()));
    agent.tools.push(Arc::new(ApprovedTool(ToolDefinition {
        name: "approved".into(),
        description: "approved tool".into(),
        input_schema: schemars::schema_for!(serde_json::Value),
        read_only: true,
        requires_approval: true,
    })));
    let mut req = request(vec![
        message("  [COMPACTION CARRY-FORWARD]\nold"),
        message("task"),
    ]);
    if let RunItem::Message { message } = &mut req.input[1] {
        message.role = Role::Assistant;
    }
    req.input_provenance = vec![
        ItemProvenance::Unattributed,
        ItemProvenance::Agent {
            name: "original".into(),
        },
    ];
    let outcome = Runner::new(agent, config)
        .unwrap()
        .run(context("markers"), req, Arc::new(HostImpl))
        .await
        .unwrap();
    let markers = journal.history_markers().unwrap();
    assert!(!markers.is_empty());
    assert!(
        markers
            .iter()
            .all(|marker| marker.marker.data.call_id == call.id
                && marker.before_item <= outcome.result.history.len())
    );
    let requests = model.requests.lock().unwrap();
    let compacted = &requests[1];
    assert_eq!(text(&compacted.input[0]), "task");
    assert_eq!(
        compacted.input_provenance[0],
        ItemProvenance::Agent {
            name: "original".into()
        }
    );
    assert_eq!(
        text(compacted.input.last().unwrap()),
        format!("{PREFIX}static state")
    );
    assert_eq!(compacted.input.len(), compacted.input_provenance.len());
    journal
        .encode_history(
            &outcome.result.history,
            &outcome
                .result
                .history_provenance
                .iter()
                .map(|source| match source {
                    ItemProvenance::Agent { name } => {
                        Some(adk_codec::dto::AgentRef { name: name.clone() })
                    }
                    _ => None,
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
}
