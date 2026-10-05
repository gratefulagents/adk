#![cfg(feature = "builder")]
use adk::{
    builder::*,
    core::*,
    providers::factory::Kind,
    runtime::{CancellationToken, HandoffInputFilter, RunnerConfig, subagent::*},
};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct Script {
    requests: Mutex<Vec<ModelRequest>>,
    responses: Mutex<VecDeque<ModelResponse>>,
    fail_model: Option<String>,
}
impl Script {
    fn new(items: Vec<ModelResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(items.into()),
            ..Default::default()
        })
    }
    fn next(&self, request: ModelRequest) -> Result<ModelResponse, Error> {
        let fail = self.fail_model.as_ref() == Some(&request.model);
        self.requests.lock().unwrap().push(request);
        if fail {
            return Err(adk::providers::error::RequestFailure::http(
                429,
                &Default::default(),
                std::time::SystemTime::now(),
            )
            .into_error());
        }
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| response(vec![])))
    }
}
fn response(items: Vec<RunItem>) -> ModelResponse {
    let end_turn = Some(!items.iter().any(|i| matches!(i, RunItem::ToolCall { .. })));
    ModelResponse {
        items,
        end_turn,
        usage: Usage::default(),
        response_id: None,
        metadata: Default::default(),
        raw: None,
        snapshot_raw: None,
        snapshot_projection: None,
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
fn message(text: &str) -> RunItem {
    RunItem::Message {
        message: Message {
            role: Role::Assistant,
            content: vec![Content::Text { text: text.into() }],
        },
    }
}
impl Model for Script {
    fn provider(&self) -> &str {
        "offline"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move { self.next(request) })
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
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(
            async move { Ok(Box::new(Events(Some(self.next(request)?))) as Box<dyn ModelStream>) },
        )
    }
}
#[derive(Default)]
struct TestHost(AtomicUsize);
impl Host for TestHost {
    fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(ApprovalDecision::Approve) })
    }
}
fn context() -> Context {
    Context {
        run_id: "catalog-handoffs".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn role(name: &str) -> RoleSpec {
    RoleSpec {
        name: name.into(),
        instructions: format!("Only {name} instructions"),
        ..Default::default()
    }
}
fn config() -> Config {
    Config {
        model: "openai/base".into(),
        instructions: "Parent instructions".into(),
        roles: vec![role("reviewer")],
        features: Some(Features {
            handoffs: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn builder(config: Config, model: &Arc<Script>) -> Builder {
    Builder::new(config)
        .model("openai", Kind::OpenAi, model.clone())
        .unwrap()
}
fn invalid<T>(result: Result<T, Error>) -> Error {
    let error = result.err().expect("expected construction failure");
    assert_eq!(error.info.category, ErrorCategory::InvalidInput);
    error
}
struct Probe {
    definition: ToolDefinition,
    calls: AtomicUsize,
}
impl Probe {
    fn new(name: &str, read_only: bool) -> Arc<Self> {
        Arc::new(Self {
            definition: ToolDefinition {
                name: name.into(),
                description: name.into(),
                input_schema: json!({"type":"object"}).try_into().unwrap(),
                read_only,
                requires_approval: false,
            },
            calls: AtomicUsize::new(0),
        })
    }
}
impl Tool for Probe {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
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
async fn actual_transfer_run_and_stream_filter_tools_and_preserve_role_only_prompt() {
    for streaming in [false, true] {
        let model = Script::new(vec![
            response(vec![call("transfer_to_reviewer")]),
            response(vec![message("reviewed")]),
        ]);
        let mut bundle = builder(config(), &model).build(&context()).await.unwrap();
        assert!(bundle.session().subagents().is_none());
        assert!(bundle.agent().tools.is_empty());
        assert_eq!(
            bundle.agent().handoffs[0].input_filter,
            HandoffInputFilter::RemoveTools
        );
        assert!(
            bundle
                .agent()
                .instructions
                .contains("does not run a nested task")
        );
        let input = vec![
            message("review"),
            call("old"),
            RunItem::ToolResult {
                call_id: "old".into(),
                output: ToolOutput {
                    content: vec![],
                    is_error: false,
                    should_pause: false,
                },
            },
        ];
        let result = if streaming {
            bundle
                .stream(context(), input, Arc::new(TestHost::default()))
                .finish()
                .await
        } else {
            bundle
                .run(context(), input, Arc::new(TestHost::default()))
                .await
        }
        .unwrap();
        assert_eq!(result.result.last_agent.as_deref(), Some("reviewer"));
        assert!(
            result
                .result
                .new_items
                .iter()
                .any(|i| matches!(i, RunItem::Handoff { agent, .. } if agent == "reviewer"))
        );
        {
            let requests = model.requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0].tools[0].name, "transfer_to_reviewer");
            assert_eq!(requests[1].instructions, "Only reviewer instructions");
            assert!(
                requests[1]
                    .input
                    .iter()
                    .all(|i| !matches!(i, RunItem::ToolCall { .. } | RunItem::ToolResult { .. }))
            );
            assert!(requests[1].tools.is_empty());
        }
        bundle.close().await.unwrap();
    }
}

#[tokio::test]
async fn feature_defaults_and_generic_fallback_are_separate_gates() {
    assert!(!Config::default().resolved_features().handoffs);
    assert!(
        !Config::default()
            .resolved_features()
            .handoff_generic_fallback
    );
    for handoffs in [false, true] {
        for generic in [false, true] {
            for catalog in [false, true] {
                let model = Script::new(vec![]);
                let mut c = config();
                if !catalog {
                    c.roles.clear();
                }
                c.features = Some(Features {
                    handoffs,
                    handoff_generic_fallback: generic,
                    ..Default::default()
                });
                let mut bundle = builder(c, &model)
                    .extra_tools([Probe::new("not_enabled", true) as Arc<dyn Tool>])
                    .build(&context())
                    .await
                    .unwrap();
                assert_eq!(
                    bundle.agent().handoffs.len(),
                    usize::from(handoffs && (catalog || generic))
                );
                assert!(bundle.agent().tools.is_empty());
                if handoffs && !catalog && generic {
                    let target = &bundle.specialists()["specialist"];
                    assert!(target.tools.is_empty());
                    assert!(target.handoffs.is_empty());
                    assert!(target.instructions.starts_with("# System context\n"));
                    assert!(target.instructions.ends_with(
                        "Resolve the delegated request and explain the result briefly."
                    ));
                    assert_eq!(
                        bundle.agent().handoffs[0].definition.description,
                        "Transfer to a specialist agent."
                    );
                }
                bundle.close().await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn target_routing_starts_from_config_not_active_parent_and_clears_fallbacks() {
    for enabled in [false, true] {
        let model = Script::new(vec![]);
        let mut c = config();
        c.fallback_models = vec!["openai/base-fallback".into()];
        c.roles = vec![
            RoleSpec {
                model_override: "openai/active".into(),
                ..role("active")
            },
            role("base"),
            RoleSpec {
                model_override: "openai/role".into(),
                fallback_models: Some(vec!["openai/role-fallback".into()]),
                ..role("routed")
            },
            RoleSpec {
                fallback_models: Some(vec![]),
                ..role("cleared")
            },
        ];
        c.active_role = Some("active".into());
        c.features.as_mut().unwrap().mode_model_routing = enabled;
        c.mode_snapshot = Some(ModeSpec {
            name: "mode".into(),
            instructions: "mode instructions".into(),
            model_routing: Some(ModelRouting {
                default_model: "openai/mode".into(),
                fallback_models: Some(vec!["openai/mode-fallback".into()]),
                role_overrides: [(
                    "routed".into(),
                    RoleRouting {
                        model: "openai/mode-role".into(),
                        fallback_models: Some(vec![]),
                        ..Default::default()
                    },
                )]
                .into(),
                ..Default::default()
            }),
            ..Default::default()
        });
        let mut bundle = builder(c, &model).build(&context()).await.unwrap();
        assert_eq!(bundle.agent().model.name(), "openai/active");
        assert_eq!(
            bundle.specialists()["base"].model.name(),
            if enabled {
                "openai/mode"
            } else {
                "openai/base"
            }
        );
        let target = &bundle.specialists()["routed"];
        assert_eq!(
            target.model.name(),
            if enabled {
                "openai/mode-role"
            } else {
                "openai/role"
            }
        );
        assert_eq!(target.instructions, "Only routed instructions");
        assert!(target.handoffs.is_empty());
        assert_eq!(target.fallbacks.len(), usize::from(!enabled));
        if !enabled {
            assert_eq!(target.fallbacks[0].name(), "openai/role-fallback");
        }
        let target = &bundle.specialists()["cleared"];
        assert_eq!(target.fallbacks.len(), usize::from(enabled));
        if enabled {
            assert_eq!(target.fallbacks[0].name(), "openai/mode-fallback");
        }
        bundle.close().await.unwrap();
    }
}

#[tokio::test]
async fn target_routes_and_fallbacks_preflight_before_resource_validation() {
    for fallback in [false, true] {
        let model = Script::new(vec![]);
        let mut c = config();
        c.features.as_mut().unwrap().tools.insert("Bash".into());
        if fallback {
            c.roles[0].fallback_models = Some(vec!["unregistered/fallback".into()]);
        } else {
            c.roles[0].model_override = "unregistered/model".into();
        }
        let error = invalid(builder(c, &model).build(&context()).await);
        assert!(
            error.info.message.contains("unknown model provider prefix"),
            "{error:?}"
        );
        assert!(model.requests.lock().unwrap().is_empty());
    }
    let model = Script::new(vec![]);
    let mut c = config();
    c.features.as_mut().unwrap().handoffs = false;
    c.roles[0].model_override = "unregistered/model".into();
    let mut bundle = builder(c, &model).build(&context()).await.unwrap();
    assert!(bundle.specialists().is_empty());
    bundle.close().await.unwrap();
}

#[derive(Clone)]
struct Catalog(Vec<RoleSpec>);
impl ConfigSource for Catalog {
    fn load<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<HostConfig, Error>> {
        Box::pin(async {
            Ok(HostConfig {
                roles: self.0.clone(),
                ..Default::default()
            })
        })
    }
}
#[tokio::test]
async fn catalogs_merge_in_source_order_and_reject_malformed_or_colliding_roles() {
    let model = Script::new(vec![]);
    for roles in [
        vec![role(" ")],
        vec![RoleSpec {
            instructions: " ".into(),
            ..role("empty")
        }],
        vec![role("same"), role("same")],
        vec![role("a-b"), role("a.b")],
        vec![role("💥"), role("??")],
    ] {
        let mut c = config();
        c.roles = roles.clone();
        c.features.as_mut().unwrap().handoff_generic_fallback = true;
        invalid(builder(c, &model).build(&context()).await);
        let mut c = config();
        c.roles.clear();
        invalid(
            builder(c, &model)
                .source(Arc::new(Catalog(roles)))
                .build(&context())
                .await,
        );
    }
    let mut c = config();
    c.roles = vec![
        RoleSpec {
            description: "replacement description".into(),
            ..role("Zed")
        },
        role("._A B-é9_."),
        role("💥"),
    ];
    let mut bundle = builder(c, &model)
        .source(Arc::new(Catalog(vec![role("Zed"), role("Alpha")])))
        .build(&context())
        .await
        .unwrap();
    assert_eq!(
        bundle
            .agent()
            .handoffs
            .iter()
            .map(|h| h.definition.name.as_str())
            .collect::<Vec<_>>(),
        [
            "transfer_to_zed",
            "transfer_to_alpha",
            "transfer_to_a_b_9",
            "transfer_to_specialist"
        ]
    );
    assert_eq!(
        bundle.agent().handoffs[0].definition.description,
        "replacement description"
    );
    assert_eq!(
        bundle.agent().handoffs[1].definition.description,
        "Transfer the conversation to the Alpha specialist."
    );
    bundle.close().await.unwrap();
}

#[tokio::test]
async fn graph_allowlists_intersect_original_host_policy_and_denial_wins() {
    for allow in [
        None,
        Some(vec![]),
        Some(vec!["inspect"]),
        Some(vec!["transfer_to_reviewer"]),
        Some(vec!["transfer_to_reviewer", "inspect"]),
    ] {
        for deny in [false, true] {
            let model = Script::new(vec![]);
            let mut c = config();
            c.features
                .as_mut()
                .unwrap()
                .tools
                .insert("ExtraTools".into());
            c.policy.tools.allowed_tools = allow
                .as_ref()
                .map(|names| names.iter().map(|n| (*n).into()).collect());
            if deny {
                c.policy
                    .tools
                    .denied_tools
                    .insert("transfer_to_reviewer".into());
            }
            let mut bundle = builder(c, &model)
                .extra_tools([Probe::new("inspect", true) as Arc<dyn Tool>])
                .build(&context())
                .await
                .unwrap();
            let names = bundle.policy().tools.allowed_tools.as_ref().unwrap();
            assert_eq!(
                names.contains("transfer_to_reviewer"),
                !deny
                    && allow
                        .as_ref()
                        .is_none_or(|a| a.contains(&"transfer_to_reviewer"))
            );
            assert_eq!(
                names.contains("inspect"),
                allow.as_ref().is_none_or(|a| a.contains(&"inspect"))
            );
            bundle
                .run(context(), vec![], Arc::new(TestHost::default()))
                .await
                .unwrap();
            assert_eq!(
                model.requests.lock().unwrap()[0]
                    .tools
                    .iter()
                    .map(|t| t.name.clone())
                    .collect::<std::collections::BTreeSet<_>>(),
                *names
            );
            bundle.close().await.unwrap();
        }
    }
}

#[tokio::test]
async fn readonly_targets_deny_mutation_exceptions_and_saved_handles_revoke_on_close() {
    for streaming in [false, true] {
        let model = Script::new(vec![
            response(vec![call("transfer_to_reviewer")]),
            response(vec![call("mutate")]),
            response(vec![message("done")]),
        ]);
        let mutate = Probe::new("mutate", false);
        let inspect = Probe::new("inspect", true);
        let mut c = config();
        c.roles[0].tool_access = "read-only".into();
        c.policy
            .tools
            .allowed_mutating_tools
            .insert("mutate".into());
        c.features
            .as_mut()
            .unwrap()
            .tools
            .insert("ExtraTools".into());
        let mut bundle = builder(c, &model)
            .extra_tools([mutate.clone() as Arc<dyn Tool>, inspect.clone()])
            .build(&context())
            .await
            .unwrap();
        assert_eq!(bundle.agent().tools.len(), 2);
        assert_eq!(bundle.policy().tools.access, AccessMode::WorkspaceWrite);
        let target = bundle.specialists()["reviewer"].clone();
        assert_eq!(target.tool_access_ceiling, Some(AccessMode::ReadOnly));
        assert_eq!(target.tools.len(), 1);
        let host = Arc::new(TestHost::default());
        if streaming {
            bundle
                .stream(context(), vec![], host.clone())
                .finish()
                .await
                .unwrap();
        } else {
            bundle.run(context(), vec![], host.clone()).await.unwrap();
        }
        assert_eq!(mutate.calls.load(Ordering::SeqCst), 0);
        assert_eq!(host.0.load(Ordering::SeqCst), 0);
        assert_eq!(
            model.requests.lock().unwrap()[1]
                .tools
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
            ["inspect"]
        );
        assert!(
            bundle
                .policy()
                .tools
                .allowed_mutating_tools
                .contains("mutate")
        );
        bundle.close().await.unwrap();
        let error = target.tools[0]
            .execute(
                &ToolContext {
                    operation: context(),
                    work_dir: ".".into(),
                    policy: ToolPolicy {
                        access: AccessMode::FullAccess,
                        ..Default::default()
                    },
                    idempotency_key: None,
                },
                ToolCall {
                    id: "saved".into(),
                    name: "inspect".into(),
                    arguments: json!({}),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.info.category, ErrorCategory::Cancelled);
        assert_eq!(inspect.calls.load(Ordering::SeqCst), 0);
    }
}

struct PendingChild;
impl ChildExecutor for PendingChild {
    fn execute<'a>(
        &'a self,
        _: ChildInvocation,
        _: ChildControl,
    ) -> BoxFuture<'a, Result<ChildOutcome, Error>> {
        Box::pin(std::future::pending())
    }
}
fn session() -> SessionState {
    SessionState::with_scheduler(
        Scheduler::new(
            context(),
            SchedulerConfig::default(),
            Arc::new(PendingChild),
            None,
        )
        .unwrap(),
    )
}
#[tokio::test]
async fn managed_subagent_feature_matrix_is_explicit_and_session_owned() {
    for mask in 0..8 {
        let selection = SubagentFeatures {
            task: mask & 1 != 0,
            status: mask & 2 != 0,
            control: mask & 4 != 0,
        };
        let mut c = config();
        c.features.as_mut().unwrap().subagents = selection.clone();
        let model = Script::new(vec![]);
        let result = builder(c.clone(), &model).build(&context()).await;
        if mask == 0 {
            result.unwrap().close().await.unwrap();
        } else {
            assert!(
                invalid(result)
                    .info
                    .message
                    .contains("session-owned scheduler")
            );
        }
        let mut owner = session();
        let mut bundle = builder(c, &model)
            .session(owner.handle())
            .extra_tools([Probe::new("not_enabled", true) as Arc<dyn Tool>])
            .build(&context())
            .await
            .unwrap();
        let actual: std::collections::BTreeSet<_> = bundle
            .agent()
            .tools
            .iter()
            .map(|t| t.definition().name.as_str())
            .collect();
        let expected: std::collections::BTreeSet<_> = [
            (selection.task, "subagent"),
            (selection.task, "subagent_wait"),
            (selection.status, "subagent_status"),
            (selection.control, "subagent_control"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
        assert_eq!(actual, expected, "mask={mask}");
        assert!(bundle.specialists()["reviewer"].tools.is_empty());
        assert!(Arc::ptr_eq(
            bundle.session().subagents().unwrap(),
            owner.handle().subagents().unwrap()
        ));
        bundle.close().await.unwrap();
        assert!(!owner.handle().is_closed());
        owner.close().await.unwrap();
    }
}

#[tokio::test]
async fn managed_subagent_selection_never_bypasses_host_name_policy() {
    let mut owner = session();
    let model = Script::new(vec![]);
    for allowed in [
        Some(std::collections::BTreeSet::new()),
        Some(["subagent_status".into(), "subagent_control".into()].into()),
        None,
    ] {
        let mut c = config();
        c.features.as_mut().unwrap().subagents = SubagentFeatures {
            status: true,
            control: true,
            ..Default::default()
        };
        c.policy.tools.allowed_tools = allowed.clone();
        c.policy
            .tools
            .denied_tools
            .insert("subagent_control".into());
        let mut bundle = builder(c, &model)
            .session(owner.handle())
            .build(&context())
            .await
            .unwrap();
        let visible: Vec<_> = bundle
            .agent()
            .tools
            .iter()
            .map(|t| t.definition().name.as_str())
            .collect();
        if allowed.as_ref().is_some_and(|a| a.is_empty()) {
            assert!(visible.is_empty());
        } else {
            assert_eq!(visible, ["subagent_status"]);
        }
        assert!(
            !bundle
                .policy()
                .tools
                .allowed_tools
                .as_ref()
                .unwrap()
                .contains("subagent_control")
        );
        bundle.close().await.unwrap();
    }
    owner.close().await.unwrap();
}

#[tokio::test]
async fn signal_and_managed_scheduler_tools_are_parent_only_and_shared_session_survives() {
    let mut session = session();
    let model = Script::new(vec![]);
    let mut c = config();
    c.features.as_mut().unwrap().subagents = SubagentFeatures {
        task: true,
        status: true,
        control: true,
    };
    c.features.as_mut().unwrap().tools = ["ExtraTools".into(), "Signals.Finish".into()].into();
    let mut bundle = builder(c, &model)
        .session(session.handle())
        .extra_tools([
            Probe::new("present_plan", true) as Arc<dyn Tool>,
            Probe::new("AskUserQuestion", true),
            Probe::new("Finish", true),
            Probe::new("inspect", true),
        ])
        .build(&context())
        .await
        .unwrap();
    let parent: Vec<_> = bundle
        .agent()
        .tools
        .iter()
        .map(|t| t.definition().name.as_str())
        .collect();
    for name in ["finish", "present_plan", "AskUserQuestion", "subagent"] {
        assert!(parent.contains(&name), "{parent:?}");
    }
    assert_eq!(
        bundle.specialists()["reviewer"]
            .tools
            .iter()
            .map(|t| t.definition().name.as_str())
            .collect::<Vec<_>>(),
        ["Finish", "inspect"]
    );
    bundle.close().await.unwrap();
    assert!(!session.handle().is_closed());
    session.close().await.unwrap();
}

#[tokio::test]
async fn collision_and_runner_errors_preserve_borrowed_and_close_owned_sessions() {
    let model = Script::new(vec![]);
    for collision in [false, true] {
        for owned in [false, true] {
            let mut state = Some(session());
            let handle = state.as_ref().unwrap().handle();
            let mut c = config();
            c.features
                .as_mut()
                .unwrap()
                .tools
                .insert("ExtraTools".into());
            let mut b = builder(c, &model);
            if collision {
                b = b.extra_tools([Probe::new("transfer_to_reviewer", true) as Arc<dyn Tool>]);
            } else {
                b = b.runner_config(RunnerConfig {
                    limits: adk::runtime::Limits {
                        max_cost: Some(-1.0),
                        ..Default::default()
                    },
                    ..Default::default()
                });
            }
            b = if owned {
                b.owned_session(state.take().unwrap())
            } else {
                b.session(handle.clone())
            };
            invalid(b.build(&context()).await);
            assert_eq!(handle.is_closed(), owned);
            if let Some(mut state) = state {
                state.close().await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn full_target_never_broadens_parent_mode_or_host_access() {
    for mode in [false, true] {
        let model = Script::new(vec![]);
        let mut c = config();
        c.roles[0].tool_access = "full-access".into();
        c.features
            .as_mut()
            .unwrap()
            .tools
            .insert("ExtraTools".into());
        if mode {
            c.active_mode = Some("plan".into());
        } else {
            c.policy.tools.access = AccessMode::ReadOnly;
        }
        let mut bundle = builder(c, &model)
            .extra_tools([Probe::new("mutate", false) as Arc<dyn Tool>])
            .build(&context())
            .await
            .unwrap();
        assert_eq!(
            bundle.specialists()["reviewer"].tool_access_ceiling,
            Some(AccessMode::ReadOnly)
        );
        assert!(bundle.specialists()["reviewer"].tools.is_empty());
        assert!(bundle.agent().tools.is_empty());
        bundle.close().await.unwrap();
    }
}

#[tokio::test]
async fn actual_target_fallback_run_and_stream_use_shared_registered_routes() {
    for streaming in [false, true] {
        let model = Arc::new(Script {
            fail_model: Some("unavailable".into()),
            responses: Mutex::new(
                vec![
                    response(vec![call("transfer_to_reviewer")]),
                    response(vec![message("fallback response")]),
                ]
                .into(),
            ),
            ..Default::default()
        });
        let mut c = config();
        c.roles[0].model_override = "openai/unavailable".into();
        c.roles[0].fallback_models = Some(vec!["openai/recovery".into()]);
        let mut bundle = builder(c, &model).build(&context()).await.unwrap();
        let outcome = if streaming {
            bundle
                .stream(context(), vec![], Arc::new(TestHost::default()))
                .finish()
                .await
        } else {
            bundle
                .run(context(), vec![], Arc::new(TestHost::default()))
                .await
        }
        .unwrap();
        assert_eq!(outcome.result.last_agent.as_deref(), Some("reviewer"));
        assert_eq!(
            model
                .requests
                .lock()
                .unwrap()
                .iter()
                .map(|r| r.model.clone())
                .collect::<Vec<_>>(),
            ["base", "unavailable", "recovery"]
        );
        bundle.close().await.unwrap();
    }
}

struct Guard;
impl adk::runtime::Guardrail for Guard {
    fn name(&self) -> &str {
        "host-guard"
    }
    fn check<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
        _: adk::runtime::GuardrailInput<'a>,
    ) -> BoxFuture<'a, Result<Option<adk::runtime::GuardrailResult>, Error>> {
        Box::pin(async { Ok(None) })
    }
}
#[tokio::test]
async fn catalog_and_generic_targets_clone_host_guardrails() {
    for generic in [false, true] {
        let model = Script::new(vec![]);
        let mut c = config();
        if generic {
            c.roles.clear();
            c.features.as_mut().unwrap().handoff_generic_fallback = true;
        }
        let guard: Arc<dyn adk::runtime::Guardrail> = Arc::new(Guard);
        let mut bundle = builder(c, &model)
            .input_guardrails([guard.clone()])
            .output_guardrails([guard.clone()])
            .build(&context())
            .await
            .unwrap();
        let target = bundle.specialists().values().next().unwrap();
        assert!(Arc::ptr_eq(&target.input_guardrails[0], &guard));
        assert!(Arc::ptr_eq(&target.output_guardrails[0], &guard));
        bundle.close().await.unwrap();
    }
}

#[tokio::test]
async fn filtered_injected_tools_still_cannot_collide_with_transfer_names() {
    let model = Script::new(vec![]);
    let mut c = config();
    c.policy.tools.access = AccessMode::ReadOnly;
    c.features
        .as_mut()
        .unwrap()
        .tools
        .insert("ExtraTools".into());
    invalid(
        builder(c.clone(), &model)
            .extra_tools([Probe::new("transfer_to_reviewer", false) as Arc<dyn Tool>])
            .build(&context())
            .await,
    );
    c.features.as_mut().unwrap().tools.clear();
    let mut bundle = builder(c, &model)
        .extra_tools([Probe::new("transfer_to_reviewer", false) as Arc<dyn Tool>])
        .build(&context())
        .await
        .unwrap();
    assert_eq!(bundle.agent().handoffs.len(), 1);
    bundle.close().await.unwrap();
}

#[tokio::test]
async fn bounded_catalog_handoff_projection_matches_independent_pinned_go_oracle() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-catalog-handoffs.json"
    ))
    .unwrap();
    assert_eq!(
        fixture["sdk_revision"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    assert_eq!(fixture["schema_version"], 1);
    // Only routing/prompt/settings and ordered handoff definitions are compared.
    // Host tools, SDK subagent gates, parent prompts and Agent serialization are outside this projection.
    let selected = [
        "two_roles_order_and_access",
        "empty_catalog_no_fallback",
        "empty_catalog_handoff_fallback",
        "sanitizer_unicode_and_punctuation",
        "role_model_and_fallback_override",
        "mode_defaults_override_role",
        "mode_role_routing_wins",
        "mode_routing_gate_off",
        "role_access_aliases",
        "blank_description_fallback",
    ];
    let mut compared = 0;
    let strings = |value: &serde_json::Value| -> Vec<String> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect()
    };
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        if !selected.contains(&name) {
            continue;
        }
        assert!(case["source_only"].as_array().unwrap().is_empty());
        let input = &case["input"];
        let mut c = Config {
            agent_name: input["agent_name"].as_str().unwrap().into(),
            instructions: input["instructions"].as_str().unwrap().into(),
            model: input["model"].as_str().unwrap().into(),
            fallback_models: strings(&input["fallback_models"]),
            reasoning: input["reasoning"].as_str().unwrap().into(),
            verbosity: input["verbosity"].as_str().unwrap().into(),
            features: Some(Features {
                handoffs: input["handoffs"]["Enabled"].as_bool().unwrap(),
                handoff_generic_fallback: input["handoffs"]["GenericFallback"].as_bool().unwrap(),
                mode_model_routing: input["mode_routing"].as_bool().unwrap(),
                parallel_tool_calls: input["parallel_tool_calls"].as_bool().unwrap(),
                ..Default::default()
            }),
            ..Default::default()
        };
        c.settings
            .insert("max_tokens".into(), input["max_tokens"].clone());
        for role in input["roles"].as_array().into_iter().flatten() {
            c.roles.push(RoleSpec {
                name: role["Name"].as_str().unwrap().into(),
                description: role["Description"].as_str().unwrap().into(),
                instructions: role["Instructions"].as_str().unwrap().into(),
                tool_access: role["ToolAccess"].as_str().unwrap().into(),
                model_override: role["ModelOverride"].as_str().unwrap().into(),
                fallback_models: (!role["FallbackModels"].is_null())
                    .then(|| strings(&role["FallbackModels"])),
            });
        }
        if !input["mode"].is_null() {
            let mode = &input["mode"]["ModelRouting"];
            let mut routing = ModelRouting {
                default_model: mode["DefaultModel"].as_str().unwrap().into(),
                fallback_models: (!mode["FallbackModels"].is_null())
                    .then(|| strings(&mode["FallbackModels"])),
                reasoning_level: mode["ReasoningLevel"].as_str().unwrap().into(),
                text_verbosity: mode["TextVerbosity"].as_str().unwrap().into(),
                ..Default::default()
            };
            for (name, role) in mode["RoleOverrides"].as_object().into_iter().flatten() {
                routing.role_overrides.insert(
                    name.clone(),
                    RoleRouting {
                        model: role["Model"].as_str().unwrap().into(),
                        fallback_models: (!role["FallbackModels"].is_null())
                            .then(|| strings(&role["FallbackModels"])),
                        reasoning_level: role["ReasoningLevel"].as_str().unwrap().into(),
                        text_verbosity: role["TextVerbosity"].as_str().unwrap().into(),
                        ..Default::default()
                    },
                );
            }
            c.mode_snapshot = Some(ModeSpec {
                model_routing: Some(routing),
                ..Default::default()
            });
        }
        let model = Script::new(vec![]);
        let mut bundle = builder(c, &model).build(&context()).await.unwrap();
        let expected: Vec<_> = case["output"]["handoffs"]
            .as_array()
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(bundle.agent().handoffs.len(), expected.len(), "{name}");
        for (actual, expected) in bundle.agent().handoffs.iter().zip(expected) {
            assert_eq!(actual.definition.name, expected["tool_name"], "{name}");
            assert_eq!(
                actual.definition.description, expected["description"],
                "{name}"
            );
            assert_eq!(
                actual.definition.read_only, expected["tool"]["read_only"],
                "{name}"
            );
            assert_eq!(actual.input_filter, HandoffInputFilter::RemoveTools);
            assert_eq!(expected["has_input_filter"], true);
            let target = &actual.target;
            assert_eq!(target.name, expected["target"]["name"], "{name}");
            assert_eq!(
                target.instructions, expected["target"]["instructions"],
                "{name}"
            );
            assert_eq!(target.model.name(), expected["target"]["model"], "{name}");
            assert_eq!(
                target
                    .fallbacks
                    .iter()
                    .map(|f| f.name().to_owned())
                    .collect::<Vec<_>>(),
                strings(&expected["target"]["fallback_models"]),
                "{name}"
            );
            assert_eq!(
                serde_json::Value::Object(target.settings.clone()),
                expected["target"]["model_settings"],
                "{name}"
            );
            if let Some(key) = expected["specialist_key"].as_str() {
                assert!(Arc::ptr_eq(target, &bundle.specialists()[key]));
            }
        }
        bundle.close().await.unwrap();
        compared += 1;
    }
    assert_eq!(compared, selected.len());
}
