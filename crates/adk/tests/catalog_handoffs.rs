#![cfg(feature = "builder")]
#[path = "builder_subagents/mod.rs"]
mod automatic_subagents;
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
            raw_arguments: None,
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
async fn builder_final_summary_selection_controls_the_last_request() {
    for enabled in [false, true] {
        let model = Script::new(vec![response(vec![message("summary")])]);
        let mut c = config();
        c.policy.max_turns = std::num::NonZeroU32::new(1).unwrap();
        c.features.as_mut().unwrap().force_final_summary_turn = enabled;
        c.features
            .as_mut()
            .unwrap()
            .tools
            .insert("ExtraTools".into());
        let mut bundle = builder(c, &model)
            .runner_config(RunnerConfig {
                force_final_summary_turn: !enabled,
                ..Default::default()
            })
            .extra_tools([Probe::new("inspect", true) as Arc<dyn Tool>])
            .build(&context())
            .await
            .unwrap();
        bundle
            .run(
                context(),
                vec![message("go")],
                Arc::new(TestHost::default()),
            )
            .await
            .unwrap();
        {
            let requests = model.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].tools.is_empty(), enabled);
            assert_eq!(requests[0].instructions.contains("<final_turn>"), enabled);
        }
        bundle.close().await.unwrap();
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
                    raw_arguments: None,
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
async fn managed_subagent_feature_matrix_matches_pinned_public_builder() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-subagent-selection.json"
    ))
    .unwrap();
    assert_eq!(
        fixture["sdk_revision"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    assert_eq!(fixture["schema_version"], 1);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    for (mask, case) in cases.iter().enumerate() {
        let selection = SubagentFeatures {
            generic_fallback: false,
            task: case["selection"]["task"].as_bool().unwrap(),
            status: case["selection"]["status"].as_bool().unwrap(),
            control: case["selection"]["control"].as_bool().unwrap(),
        };
        assert_eq!(case["scheduler"], mask != 0);
        let mut c = config();
        c.features.as_mut().unwrap().subagents = selection;
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
        let expected: std::collections::BTreeSet<_> = case["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap())
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
            generic_fallback: false,
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
        generic_fallback: false,
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
                target.handoff_description, expected["target"]["handoff_description"],
                "{name}"
            );
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

struct CompletedChild;
impl ChildExecutor for CompletedChild {
    fn execute<'a>(
        &'a self,
        invocation: ChildInvocation,
        _: ChildControl,
    ) -> BoxFuture<'a, Result<ChildOutcome, Error>> {
        Box::pin(async move { Ok(ChildOutcome::completed(invocation.agent_name)) })
    }
}

#[tokio::test]
async fn managed_subagent_default_uses_registered_catalog_not_parent_identity() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/run-instructions/observations.json"
    ))
    .unwrap();
    for case in fixture["subagent_default_cases"].as_array().unwrap() {
        let names: Vec<_> = case["names"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|name| name.as_str().unwrap())
            .collect();
        let expected = case["default"].as_str().unwrap();
        for explicit in [None, names.iter().copied().find(|name| !name.is_empty())] {
            let scheduler = Scheduler::new(
                context(),
                SchedulerConfig {
                    agents: names
                        .iter()
                        .map(|name| ((*name).to_owned(), SecurityBaseline::default()))
                        .collect(),
                    ..Default::default()
                },
                Arc::new(CompletedChild),
                None,
            )
            .unwrap();
            let mut owner = SessionState::with_scheduler(scheduler);
            let handle = owner.handle();
            let model = Script::new(vec![]);
            let mut c = config();
            c.agent_name = "not-a-registered-child".into();
            c.features.as_mut().unwrap().subagents.task = true;
            let mut bundle = builder(c, &model)
                .session(handle.clone())
                .build(&context())
                .await
                .unwrap();
            let tool = bundle
                .agent()
                .tools
                .iter()
                .find(|tool| tool.definition().name == "subagent")
                .unwrap();
            let mut arguments = json!({"message":"delegate", "mode":"sync"});
            if let Some(name) = explicit {
                arguments["agent_name"] = name.into();
            }
            let result = tool
                .execute(
                    &ToolContext {
                        operation: context(),
                        work_dir: ".".into(),
                        policy: ToolPolicy::default(),
                        idempotency_key: None,
                    },
                    ToolCall {
                        raw_arguments: None,
                        id: "delegate".into(),
                        name: "subagent".into(),
                        arguments,
                    },
                )
                .await;
            let expected = explicit.unwrap_or(expected);
            let tasks = handle.subagents().unwrap().scheduler.list();
            if names.contains(&expected) {
                assert!(
                    !result
                        .unwrap_or_else(|e| panic!("{case:?}: {e:?}"))
                        .is_error
                );
                assert_eq!(tasks.len(), 1, "{case:?}");
                assert_eq!(tasks[0].agent_name, expected, "{case:?}");
                assert_eq!(tasks[0].status, TaskStatus::Completed);
                assert_eq!(tasks[0].result, expected);
            } else {
                assert!(result.is_err(), "{case:?}");
                assert!(tasks.is_empty());
            }
            bundle.close().await.unwrap();
            assert!(!handle.is_closed());
            owner.close().await.unwrap();
        }
    }
}

struct SchemaParser {
    reject: bool,
    calls: AtomicUsize,
}
impl adk::runtime::OutputParser for SchemaParser {
    fn parse(&self, raw: &str) -> Result<serde_json::Value, Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.reject {
            Err(Error::new(
                ErrorCategory::InvalidInput,
                "parser rejected output",
            ))
        } else {
            Ok(json!({"parsed":raw}))
        }
    }
}

#[tokio::test]
async fn builder_output_schema_requests_and_results_match_pinned_sdk() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/run-instructions/observations.json"
    ))
    .unwrap();
    for case in fixture["output_schema_cases"].as_array().unwrap() {
        let schema = case["schema_json"].as_str().unwrap();
        let parser = Arc::new(SchemaParser {
            reject: case["parser"] == "reject",
            calls: AtomicUsize::new(0),
        });
        let model = Script::new(vec![response(vec![message(
            case["answer"].as_str().unwrap(),
        )])]);
        let mut c = config();
        c.output_schema = if schema.is_empty() {
            None
        } else {
            Some(serde_json::from_str(schema).unwrap())
        };
        c.output_schema_name = case["name"].as_str().unwrap().into();
        c.output_schema_strict = case["strict"].as_bool().unwrap();
        if case["parser"] != "" {
            c.output_parser = Some(parser.clone());
        }
        let mut bundle = builder(c, &model).build(&context()).await.unwrap();
        assert_eq!(
            bundle.agent().output_schema.is_some(),
            case["parent_has_schema"].as_bool().unwrap()
        );
        assert_eq!(
            bundle.specialists()["reviewer"].output_schema.is_some(),
            case["specialist_has_schema"].as_bool().unwrap()
        );
        assert_eq!(
            bundle.agent().handoffs[0].target.output_schema.is_some(),
            case["handoff_has_schema"].as_bool().unwrap()
        );
        assert!(bundle.specialists()["reviewer"].output_parser.is_none());
        let host = Arc::new(TestHost::default());
        let result = if case["streaming"] == true {
            bundle
                .stream(context(), vec![], host)
                .finish()
                .await
                .unwrap()
        } else {
            bundle.run(context(), vec![], host).await.unwrap()
        };
        assert_eq!(
            result.result.final_output,
            Some(case["final_output"].clone()),
            "{case:?}"
        );
        assert_eq!(
            parser.calls.load(Ordering::SeqCst),
            case["parser_calls"].as_u64().unwrap() as usize
        );
        {
            let requests = model.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            let request = &requests[0];
            assert_eq!(
                request.output_schema.as_ref().map(|s| s.as_value()),
                (!case["request_schema"].is_null()).then_some(&case["request_schema"])
            );
            if request.output_schema.is_some() {
                assert_eq!(
                    request.output_schema_name,
                    case["request_name"].as_str().unwrap()
                );
                assert_eq!(
                    request.output_schema_strict,
                    case["request_strict"].as_bool().unwrap()
                );
                assert!(request.instructions.contains("<structured_output>"));
                assert_eq!(
                    request.instructions.contains("Strict mode:"),
                    request.output_schema_strict
                );
            } else {
                assert!(!request.instructions.contains("<structured_output>"));
            }
        }
        bundle.close().await.unwrap();
    }
}

#[tokio::test]
async fn builder_invalid_output_schema_closes_owned_session_without_dispatch() {
    let model = Script::new(vec![]);
    let owner = SessionState::new();
    let handle = owner.handle();
    let mut c = config();
    c.output_schema = Some(json!({"type":27}).try_into().unwrap());
    let error = invalid(
        builder(c.clone(), &model)
            .owned_session(owner)
            .build(&context())
            .await,
    );
    assert!(error.info.message.contains("invalid JSON schema"));
    assert!(handle.is_closed());
    assert!(model.requests.lock().unwrap().is_empty());
    builder(c, &model)
        .build_tools(&context())
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
}

#[tokio::test]
async fn handoff_history_builder_feature_and_explicit_policy_match_pinned_selection() {
    use adk::runtime::compaction::*;
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff-compaction/observations.json"
    ))
    .unwrap();
    assert_eq!(fixture["builder"].as_array().unwrap().len(), 10);
    for case in fixture["builder"].as_array().unwrap() {
        let mode = case["mode"].as_str().unwrap();
        let mut cfg = config();
        cfg.enable_compaction = case["legacy"] == true;
        cfg.features = if mode == "legacy" {
            None
        } else {
            Some(Features {
                handoff_history: mode == "on",
                ..Default::default()
            })
        };
        if mode.starts_with("explicit-") {
            cfg.handoff_history = Some(HandoffHistoryPolicy {
                enabled: mode == "explicit-on",
                max_tokens: 300,
                target_tokens: 120,
                preserve_recent_items: 3,
                summary_bullet_limit: 2,
            });
        }
        let features = cfg.resolved_features();
        if !mode.starts_with("explicit-") {
            assert_eq!(features.handoff_history, case["policy"]["Enabled"]);
        }
        cfg.features = Some(Features {
            handoffs: true,
            compaction: false,
            ..features
        });
        let transfer = call("transfer_to_reviewer");
        let model = Script::new(vec![
            response(vec![transfer]),
            response(vec![message("done")]),
        ]);
        let mut bundle = builder(cfg, &model).build(&context()).await.unwrap();
        let mut input = vec![RunItem::Message {
            message: Message {
                role: Role::User,
                content: vec![Content::Text {
                    text: "original task".into(),
                }],
            },
        }];
        input.extend((0..24).map(|_| message(&"old context. ".repeat(100))));
        let before = input.clone();
        let expected_policy = &case["policy"];
        let policy = HandoffHistoryPolicy {
            enabled: expected_policy["Enabled"].as_bool().unwrap(),
            max_tokens: expected_policy["MaxTokens"].as_u64().unwrap(),
            target_tokens: expected_policy["TargetTokens"].as_u64().unwrap(),
            preserve_recent_items: expected_policy["PreserveRecentItems"].as_u64().unwrap()
                as usize,
            summary_bullet_limit: expected_policy["SummaryBulletLimit"].as_u64().unwrap() as usize,
        };
        let expected = compact_handoff_history(&before, &[], policy);
        let expected = if expected.changed {
            let mut history = finalize_local_history(&expected.history, &before);
            history.push(RunItem::Message { message: Message { role: Role::User, content: vec![Content::Text { text: "[COMPACTION CARRY-FORWARD]\nThis live runtime state was injected after context compaction. Treat it as current and higher priority than older compacted history.\n\nRuntime state: provider=openai, mode=chat".into() }] } });
            history
        } else {
            expected.history
        };
        let outcome = bundle
            .run(context(), input, Arc::new(TestHost::default()))
            .await
            .unwrap();
        assert_eq!(outcome.result.last_agent.as_deref(), Some("reviewer"));
        assert_eq!(model.requests.lock().unwrap()[1].input, expected, "{mode}");
        bundle.close().await.unwrap();
    }
}
