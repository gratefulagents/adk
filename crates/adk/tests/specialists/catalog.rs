use super::TestTool;
use adk::{
    core::*,
    runtime::{subagent::*, *},
    specialists::*,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, atomic::Ordering},
};

fn project(spec: &SpecialistAgentSpec) -> Value {
    json!({"name":spec.name, "instructions":spec.instructions, "description":spec.description,
        "model":spec.model, "fallbacks":spec.fallback_models, "settings":spec.settings,
        "tools":spec.tools.iter().map(|t| t.definition().name.clone()).collect::<Vec<_>>()})
}

fn omit_null_fields(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            fields.retain(|_, v| !v.is_null());
            for v in fields.values_mut() {
                omit_null_fields(v);
            }
        }
        Value::Array(values) => {
            for v in values {
                omit_null_fields(v);
            }
        }
        _ => {}
    }
}

#[derive(Default)]
struct ModelProbe(Mutex<Vec<ModelRequest>>);
impl Model for ModelProbe {
    fn provider(&self) -> &str {
        "test"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        self.0.lock().unwrap().push(request);
        Box::pin(async {
            Ok(ModelResponse {
                snapshot_raw: None,
                snapshot_projection: None,
                raw: None,
                items: vec![RunItem::Message {
                    message: Message {
                        role: Role::Assistant,
                        content: vec![Content::Text {
                            text: "child answer".into(),
                        }],
                    },
                }],
                usage: Usage::default(),
                end_turn: Some(true),
                response_id: None,
                metadata: Default::default(),
            })
        })
    }
}
struct UnusedExecutor;
impl ChildExecutor for UnusedExecutor {
    fn execute<'a>(
        &'a self,
        _: ChildInvocation,
        _: ChildControl,
    ) -> BoxFuture<'a, Result<ChildOutcome, Error>> {
        panic!("metadata preparation must not execute children")
    }
}
fn context() -> Context {
    Context {
        run_id: "catalog".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}

#[tokio::test]
async fn catalog_matches_independent_pinned_go_observations() {
    let mut fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/specialist-catalog/observations.json"
    ))
    .unwrap();
    assert_eq!(
        fixture["provenance"]["commit"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    omit_null_fields(&mut fixture);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 54);
    let owner = Scheduler::new(
        context(),
        SchedulerConfig::default(),
        Arc::new(UnusedExecutor),
        None,
    )
    .unwrap();
    let session = Arc::new(SubagentSession::new(owner.handle()));
    for case in cases {
        let input = &case["input"];
        let roles: Vec<SpecialistRoleSpec> =
            serde_json::from_value(input["Catalog"].clone()).unwrap();
        let read = TestTool::new("read", true, None);
        let write = TestTool::new("write", false, None);
        let signal = TestTool::new("finish", true, None);
        let rename = TestTool::new("rename", false, Some(signal.clone()));
        let mut options = SpecialistCatalogOptions {
            tools: vec![
                read.clone(),
                write.clone(),
                rename.clone(),
                read.clone(),
                signal.clone(),
            ],
            base_model: input["BaseModel"].as_str().unwrap().into(),
            fallback_models: serde_json::from_value(
                input.get("FallbackModels").cloned().unwrap_or(json!([])),
            )
            .unwrap(),
            provider: input["Provider"].as_str().unwrap().into(),
            base_settings: serde_json::from_value(input["BaseSettings"].clone()).unwrap(),
            routing: input
                .get("Routing")
                .map(|v| serde_json::from_value(v.clone()).unwrap()),
        };
        if input["NoTools"] == true {
            options.tools.clear();
        }
        assert_eq!(
            json!(
                build_specialists_from_catalog(&roles)
                    .iter()
                    .map(project)
                    .collect::<Vec<_>>()
            ),
            case["raw"],
            "raw {}",
            input["Name"]
        );
        let model = Arc::new(ModelProbe::default());
        let (catalog, wrappers) = if input["NoRunner"] == true {
            build_specialist_tools_from_catalog(&roles, &options, None, |_| {
                panic!("missing runner must not bind")
            })
            .unwrap()
        } else {
            let prepared = prepare_specialist_catalog(&roles, &options);
            let actual: BTreeMap<_, _> = prepared
                .iter()
                .map(|s| (s.name.clone(), project(s)))
                .collect();
            assert_eq!(json!(actual), case["agents"], "{}", input["Name"]);
            for spec in &prepared {
                for tool in &spec.tools {
                    if tool.definition().name == "read" {
                        let expected: Arc<dyn Tool> = read.clone();
                        assert!(Arc::ptr_eq(tool, &expected));
                    }
                }
            }
            let bound = bind_specialist_catalog(&prepared, |name| {
                Ok(ModelBinding::complete(name, model.clone()))
            })
            .unwrap();
            let tools = bound.attach_tools(session.clone());
            (bound, tools)
        };
        assert_eq!(
            catalog.agents().len(),
            case["agents"].as_object().unwrap().len()
        );
        for agent in catalog.agents().values() {
            assert!(agent.input_guardrails.is_empty() && agent.output_guardrails.is_empty());
            assert!(agent.tool_access_ceiling.is_none());
            assert_eq!(
                agent.model.name(),
                case["agents"][&agent.name]["model"].as_str().unwrap()
            );
            assert_eq!(
                json!(
                    agent
                        .fallbacks
                        .iter()
                        .map(ModelBinding::name)
                        .collect::<Vec<_>>()
                ),
                case["agents"][&agent.name]["fallbacks"]
            );
            assert_eq!(
                json!(agent.settings),
                case["agents"][&agent.name]["settings"]
            );
        }
        assert_eq!(json!(wrappers.iter().map(|tool| {
            let d=tool.definition();
            json!({"name":d.name,"description":d.description,"readOnly":d.read_only,"approval":d.requires_approval})
        }).collect::<Vec<_>>()),case["wrappers"],"{}",input["Name"]);
        assert_eq!(
            rename.adapter_calls.load(Ordering::SeqCst),
            case["adapterCalls"].as_array().unwrap().len()
        );
        assert!(model.0.lock().unwrap().is_empty());
        assert_eq!(
            options.tools.len(),
            if input["NoTools"] == true { 0 } else { 5 }
        );
    }
    owner.shutdown().await.unwrap();
}

struct HostProbe;
impl Host for HostProbe {
    fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { Ok(ApprovalDecision::Deny) })
    }
}

#[tokio::test]
async fn caller_owned_registration_attachment_extraction_and_shutdown() {
    for (extracted, expected) in [("summary", "summary"), ("", "child answer")] {
        let model = Arc::new(ModelProbe::default());
        let roles = vec![SpecialistRoleSpec {
            name: " worker ".into(),
            instructions: "catalog instructions".into(),
            ..Default::default()
        }];
        let prepared = prepare_specialist_catalog(
            &roles,
            &SpecialistCatalogOptions {
                base_model: "fake".into(),
                ..Default::default()
            },
        );
        let catalog = bind_specialist_catalog(&prepared, |name| {
            Ok(ModelBinding::complete(name, model.clone()))
        })
        .unwrap();
        let runners = catalog
            .agents()
            .iter()
            .map(|(name, agent)| {
                (
                    name.clone(),
                    Runner::new((**agent).clone(), RunnerConfig::default()).unwrap(),
                )
            })
            .collect();
        let executor = RunnerChildExecutor::new(runners, Arc::new(HostProbe))
            .with_output_extractor("worker", Arc::new(move |_| extracted.into()));
        let security = SecurityBaseline::default();
        let owner = Scheduler::new(
            context(),
            SchedulerConfig {
                agents: catalog
                    .agents()
                    .keys()
                    .map(|name| (name.clone(), security.clone()))
                    .collect(),
                security,
                ..Default::default()
            },
            Arc::new(executor),
            None,
        )
        .unwrap();
        let session = Arc::new(SubagentSession::new(owner.handle()));
        let tools = catalog.attach_tools(session);
        assert_eq!(tools[0].definition().name, "agent_worker");
        assert!(tools[0].definition().read_only);
        let output = tools[0]
            .execute(
                &ToolContext {
                    operation: context(),
                    policy: ToolPolicy::default(),
                    work_dir: ".".into(),
                    idempotency_key: None,
                },
                ToolCall {
                    id: "delegate".into(),
                    name: "agent_worker".into(),
                    arguments: json!({"message":"bounded task packet"}),
                    raw_arguments: None,
                },
            )
            .await
            .unwrap();
        assert!(!output.is_error, "{output:?}");
        assert_eq!(
            output.content,
            vec![Content::Text {
                text: expected.into()
            }]
        );
        {
            let requests = model.0.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert!(
                requests[0]
                    .instructions
                    .starts_with("catalog instructions\n")
            );
            assert!(
                serde_json::to_string(&requests[0].input)
                    .unwrap()
                    .contains("bounded task packet")
            );
        }
        owner.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn original_catalog_applies_access_and_routing() {
    let owner = Scheduler::new(
        context(),
        SchedulerConfig::default(),
        Arc::new(UnusedExecutor),
        None,
    )
    .unwrap();
    let session = Arc::new(SubagentSession::new(owner.handle()));
    let model = Arc::new(ModelProbe::default());
    let roles = [SpecialistRoleSpec {
        name: "researcher".into(),
        description: "Research role".into(),
        instructions: "Research carefully.".into(),
        tool_access: "analysis".into(),
        ..Default::default()
    }];
    let options = SpecialistCatalogOptions {
        tools: vec![
            TestTool::new("Write", false, None),
            TestTool::new("Read", true, None),
            TestTool::new("finish", false, None),
        ],
        base_model: "base-model".into(),
        provider: "openai".into(),
        routing: Some(SpecialistModeRouting {
            fallback_models: vec!["anthropic/claude-sonnet-4-6".into()],
            role_overrides: BTreeMap::from([(
                "researcher".into(),
                SpecialistRoleRouting {
                    model: "role-model".into(),
                    fallback_models: vec!["copilot/gpt-4.1".into()],
                    ..Default::default()
                },
            )]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let (catalog, tools) =
        build_specialist_tools_from_catalog(&roles, &options, Some(session), |name| {
            Ok(ModelBinding::complete(name, model.clone()))
        })
        .unwrap();
    let agent = &catalog.agents()["researcher"];
    assert_eq!(agent.model.name(), "role-model");
    assert_eq!(
        agent
            .fallbacks
            .iter()
            .map(ModelBinding::name)
            .collect::<Vec<_>>(),
        ["copilot/gpt-4.1"]
    );
    assert_eq!(
        agent
            .tools
            .iter()
            .map(|tool| tool.definition().name.as_str())
            .collect::<Vec<_>>(),
        ["Read"]
    );
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].definition().name, "agent_researcher");
    owner.shutdown().await.unwrap();
}

#[cfg(feature = "builder")]
#[test]
fn mode_routing_conversion_matches_pinned_go_and_owns_its_fields() {
    use adk::builder::{ModeSpec, ModelRouting, RoleRouting};
    let mut fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/specialist-catalog/observations.json"
    ))
    .unwrap();
    omit_null_fields(&mut fixture);
    let cases = fixture["routing_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 6);
    for case in cases {
        let routing: Option<SpecialistModeRouting> =
            serde_json::from_value(case["input"]["ModelRouting"].clone()).unwrap();
        let mut spec = (!case["input"].is_null()).then(|| ModeSpec {
            model_routing: routing.map(|routing| ModelRouting {
                default_model: routing.default_model,
                fallback_models: Some(routing.fallback_models),
                reasoning_level: routing.reasoning_level,
                text_verbosity: routing.text_verbosity,
                role_overrides: routing
                    .role_overrides
                    .into_iter()
                    .map(|(name, role)| {
                        (
                            name,
                            RoleRouting {
                                model: role.model,
                                fallback_models: Some(role.fallback_models),
                                reasoning_level: role.reasoning_level,
                                text_verbosity: role.text_verbosity,
                                settings: serde_json::from_value(json!({"native-only": true}))
                                    .unwrap(),
                            },
                        )
                    })
                    .collect(),
                settings: serde_json::from_value(json!({"native-only": true})).unwrap(),
            }),
            ..Default::default()
        });
        let actual = mode_routing_from_template_spec(spec.as_ref());
        let expected: Option<SpecialistModeRouting> =
            serde_json::from_value(case["output"].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        if let Some(routing) = spec.as_mut().and_then(|spec| spec.model_routing.as_mut()) {
            routing.default_model = "mutated".into();
            routing
                .fallback_models
                .as_mut()
                .unwrap()
                .push("mutated".into());
            for role in routing.role_overrides.values_mut() {
                role.model = "mutated".into();
                role.fallback_models
                    .as_mut()
                    .unwrap()
                    .push("mutated".into());
            }
            routing.role_overrides.clear();
        }
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
    assert!(mode_routing_from_template_spec(None).is_none());
    assert!(mode_routing_from_template_spec(Some(&ModeSpec::default())).is_none());
    let spec = ModeSpec {
        model_routing: Some(ModelRouting {
            role_overrides: [("empty".into(), RoleRouting::default())].into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let routing = mode_routing_from_template_spec(Some(&spec)).unwrap();
    assert!(routing.fallback_models.is_empty());
    assert!(routing.role_overrides["empty"].fallback_models.is_empty());
}
