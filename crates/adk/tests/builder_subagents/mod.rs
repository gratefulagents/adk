use super::*;
mod lifecycle;
use std::num::NonZeroU32;

#[tokio::test]
async fn automatic_catalog_and_managed_tools_match_pinned_builder_stages() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/run-instructions/observations.json"
    ))
    .unwrap();
    for case in fixture["auto_subagent_cases"].as_array().unwrap() {
        let mask = case["mask"].as_u64().unwrap();
        let roles = case["roles"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| RoleSpec {
                name: r["Name"].as_str().unwrap().into(),
                instructions: r["Instructions"].as_str().unwrap().into(),
                model_override: r["ModelOverride"].as_str().unwrap().into(),
                tool_access: r["ToolAccess"].as_str().unwrap().into(),
                ..Default::default()
            })
            .collect();
        let model = Script::new(vec![response(vec![message("done")])]);
        let mut bundle = builder(
            Config {
                model: "openai/base".into(),
                roles,
                features: Some(Features {
                    subagents: SubagentFeatures {
                        task: mask & 1 != 0,
                        status: mask & 2 != 0,
                        control: mask & 4 != 0,
                        generic_fallback: case["generic"].as_bool().unwrap(),
                    },
                    handoffs: case["handoffs"].as_bool().unwrap(),
                    handoff_generic_fallback: case["handoffs"].as_bool().unwrap(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            &model,
        )
        .subagent_host(Arc::new(TestHost::default()))
        .build(&context())
        .await
        .unwrap();
        let actual: Vec<_> = bundle
            .agent()
            .tools
            .iter()
            .map(|tool| tool.definition().name.clone())
            .collect();
        assert_eq!(json!(actual), case["tools"], "{case:?}");
        let handle = bundle.session();
        let children = handle.subagents();
        assert_eq!(
            children.is_some(),
            case["scheduler"].as_bool().unwrap(),
            "{case:?}"
        );
        if let Some(children) = children {
            let registered: Vec<_> = children.scheduler.agent_names().collect();
            let expected: Vec<_> = case["agents"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(registered, expected);
        }
        for (name, expected) in case["agents"].as_object().unwrap() {
            let agent = &bundle.specialists()[name];
            assert_eq!(agent.model.name(), expected["model"]);
            assert_eq!(agent.instructions, expected["instructions"]);
            if mask & 1 != 0 {
                assert!(
                    bundle
                        .agent()
                        .instructions
                        .contains(expected["guide_line"].as_str().unwrap())
                );
            } else {
                assert!(
                    !bundle
                        .agent()
                        .instructions
                        .contains("Available specialist sub-agents")
                );
            }
            assert_eq!(
                agent.tools.len(),
                expected["tools"].as_u64().unwrap() as usize
            );
        }
        if let Some(tool) = bundle
            .agent()
            .tools
            .iter()
            .find(|tool| tool.definition().name == "subagent")
        {
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
                        id: "fixture-call".into(),
                        name: "subagent".into(),
                        arguments: json!({"message":"delegate","mode":"sync"}),
                    },
                )
                .await
                .unwrap();
            assert!(!result.is_error, "{case:?}: {result:?}");
            let tasks = children.unwrap().scheduler.list();
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0].agent_name, case["task_agent"]);
            assert_eq!(
                serde_json::to_value(tasks[0].status).unwrap(),
                case["task_status"]
            );
        }
        assert_eq!(
            model.requests.lock().unwrap().len(),
            case["requests"].as_u64().unwrap() as usize
        );
        let handle = bundle.session().clone();
        bundle.close().await.unwrap();
        assert!(handle.is_closed());
    }
}

#[tokio::test]
async fn automatic_children_run_in_both_modes_with_role_security_and_limits() {
    for streaming in [false, true] {
        for mode in ["sync", "background"] {
            let parent = Script::new(vec![
                response(vec![RunItem::ToolCall {
                    call: ToolCall {
                        raw_arguments: None,
                        id: "delegate".into(),
                        name: "subagent".into(),
                        arguments: json!({"message":"review","mode":mode}),
                    },
                }]),
                response(vec![message("parent done")]),
                response(vec![message("parent done")]),
            ]);
            let child = Script::new(vec![response(vec![message("child done")])]);
            let mutate = Probe::new("mutate", false);
            let inspect = Probe::new("inspect", true);
            let mut c = config();
            c.roles[0].model_override = "worker/review".into();
            c.roles[0].tool_access = "read-only".into();
            c.features.as_mut().unwrap().handoffs = false;
            c.features.as_mut().unwrap().subagents.task = true;
            c.features.as_mut().unwrap().tools =
                ["ExtraTools".into(), "Signals.Finish".into()].into();
            c.policy
                .tools
                .allowed_mutating_tools
                .insert("mutate".into());
            c.mode_snapshot = Some(ModeSpec {
                constraints: Some(Constraints {
                    max_concurrent_subagents: NonZeroU32::new(1),
                    subagent_max_turns: NonZeroU32::new(2),
                    ..Default::default()
                }),
                ..Default::default()
            });
            c.output_schema = Some(true.into());
            let host = Arc::new(TestHost::default());
            let mut bundle = builder(c, &parent)
                .model("worker", Kind::Local, child.clone())
                .unwrap()
                .extra_tools([mutate.clone() as Arc<dyn Tool>, inspect])
                .subagent_host(host.clone())
                .runner_config(RunnerConfig {
                    additional_instructions: "PARENT-ONLY".into(),
                    ..Default::default()
                })
                .build(&context())
                .await
                .unwrap();
            assert!(bundle.agent().handoffs.is_empty());
            let scheduler = bundle.session().subagents().unwrap().scheduler.clone();
            assert_eq!(scheduler.max_concurrency(), 1);
            let outcome = if streaming {
                bundle
                    .stream(context(), vec![], host)
                    .finish()
                    .await
                    .unwrap()
            } else {
                bundle.run(context(), vec![], host).await.unwrap()
            };
            assert_eq!(outcome.result.final_output, Some(json!("parent done")));
            {
                let requests = child.requests.lock().unwrap();
                assert_eq!(requests.len(), 1);
                assert_eq!(requests[0].model, "review");
                assert!(
                    requests[0]
                        .instructions
                        .contains("Only reviewer instructions")
                );
                assert!(!requests[0].instructions.contains("PARENT-ONLY"));
                assert!(requests[0].output_schema.is_none());
                assert_eq!(
                    requests[0]
                        .tools
                        .iter()
                        .map(|t| t.name.as_str())
                        .collect::<Vec<_>>(),
                    ["inspect"]
                );
            }
            assert_eq!(mutate.calls.load(Ordering::SeqCst), 0);
            let checkpoint = scheduler.snapshot();
            assert_eq!(checkpoint.records.len(), 1);
            assert_eq!(checkpoint.records[0].submission.policy.max_turns.get(), 2);
            assert_eq!(
                checkpoint.records[0].security_baseline.tools.access,
                AccessMode::ReadOnly
            );
            assert_eq!(scheduler.list()[0].status, TaskStatus::Completed);
            bundle.close().await.unwrap();
            assert!(
                scheduler
                    .submit(Submission::new("reviewer", "after close"))
                    .await
                    .is_err()
            );
        }
    }
}

#[tokio::test]
async fn automatic_children_do_not_mutate_borrowed_sessions_or_replace_schedulers() {
    let mut owner = SessionState::new();
    let model = Script::new(vec![]);
    let mut c = config();
    c.features.as_mut().unwrap().subagents.task = true;
    let error = invalid(
        builder(c.clone(), &model)
            .session(owner.handle())
            .subagent_host(Arc::new(TestHost::default()))
            .build(&context())
            .await,
    );
    assert!(error.info.message.contains("owned session"));
    assert!(!owner.handle().is_closed());
    owner.close().await.unwrap();

    let mut existing = session();
    let handle = existing.handle();
    let host = Arc::new(TestHost::default());
    let mut bundle = builder(c, &model)
        .session(handle.clone())
        .subagent_host(host.clone())
        .build(&context())
        .await
        .unwrap();
    assert!(Arc::ptr_eq(
        bundle.session().subagents().unwrap(),
        handle.subagents().unwrap()
    ));
    assert_eq!(Arc::strong_count(&host), 1);
    bundle.close().await.unwrap();
    assert!(!handle.is_closed());
    existing.close().await.unwrap();
}

#[tokio::test]
async fn automatic_children_release_owned_graph_on_failed_parent_validation() {
    let owner = SessionState::new();
    let handle = owner.handle();
    let model = Script::new(vec![]);
    let host = Arc::new(TestHost::default());
    let mut c = config();
    c.features.as_mut().unwrap().subagents.task = true;
    c.output_schema = Some(json!({"type":27}).try_into().unwrap());
    invalid(
        builder(c, &model)
            .owned_session(owner)
            .subagent_host(host.clone())
            .build(&context())
            .await,
    );
    assert!(handle.is_closed());
    assert_eq!(Arc::strong_count(&host), 1);
    assert_eq!(Arc::strong_count(&model), 1);
    assert!(model.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn automatic_delegation_guidance_respects_parent_tool_policy() {
    let model = Script::new(vec![]);
    let mut c = config();
    c.features.as_mut().unwrap().handoffs = false;
    c.features.as_mut().unwrap().subagents.task = true;
    c.policy.tools.denied_tools.insert("subagent".into());
    let mut bundle = builder(c, &model)
        .subagent_host(Arc::new(TestHost::default()))
        .build(&context())
        .await
        .unwrap();
    assert!(bundle.session().subagents().is_some());
    assert!(
        !bundle
            .agent()
            .tools
            .iter()
            .any(|tool| tool.definition().name == "subagent")
    );
    assert!(
        !bundle
            .agent()
            .instructions
            .contains("Available specialist sub-agents")
    );
    bundle.close().await.unwrap();
}
