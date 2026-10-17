use super::*;

struct PromptTool(ToolDefinition);
impl Tool for PromptTool {
    fn definition(&self) -> &ToolDefinition {
        &self.0
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        panic!("prompt fixture must not execute tools")
    }
}
fn tool(name: &str, read_only: bool) -> Arc<dyn Tool> {
    Arc::new(PromptTool(ToolDefinition {
        name: name.into(),
        description: "prompt fixture".into(),
        input_schema: json!({"type":"object"}).try_into().unwrap(),
        read_only,
        requires_approval: false,
    }))
}

#[tokio::test]
async fn prepared_workspace_blocks_match_pinned_sdk_and_reach_run_and_stream_requests() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/workspace-context/observations.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let names: Vec<String> = serde_json::from_value(case["tools"].clone()).unwrap();
        if names.windows(2).any(|pair| pair[0] > pair[1]) {
            continue;
        }
        let strict = case["strict"].as_bool().unwrap();
        let access = if case["access"] == "read-only" {
            AccessMode::ReadOnly
        } else {
            AccessMode::WorkspaceWrite
        };
        let mut config = Config {
            instructions: "host instructions".into(),
            work_dir: case["work_dir"].as_str().unwrap().into(),
            features: strict.then(|| Features {
                tools: ["ExtraTools".into()].into(),
                mode_instructions: case["mode_instructions"].as_bool().unwrap_or(false),
                ..Default::default()
            }),
            legacy_tools: adk::tools::LegacyFeatures {
                enable_subagents: true,
                ..Default::default()
            },
            ..Default::default()
        };
        config.active_mode = case["active_mode"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if let Some(snapshot) = case["mode_snapshot"].as_object() {
            config.mode_snapshot = Some(ModeSpec {
                name: snapshot["name"].as_str().unwrap().into(),
                display_name: snapshot["display_name"].as_str().unwrap().into(),
                tool_access: "full".into(),
                ..Default::default()
            });
        }
        config.policy.tools.access = access;
        let model = Arc::new(RecordingModel::default());
        let mut bundle = builder(config, &model)
            .extra_tools(names.iter().map(|n| tool(n, true)))
            .build(&context())
            .await
            .unwrap();
        assert_eq!(
            bundle.agent().instructions,
            case["instructions"].as_str().unwrap(),
            "{case}"
        );
        bundle
            .run(context(), vec![], Arc::new(TestHost))
            .await
            .unwrap();
        bundle
            .stream(context(), vec![], Arc::new(TestHost))
            .finish()
            .await
            .unwrap();
        for request in model.requests.lock().unwrap().iter() {
            assert_eq!(request.instructions, bundle.agent().instructions);
        }
        bundle.close().await.unwrap();
    }
}

#[tokio::test]
async fn workspace_prompt_uses_prepared_names_and_narrowed_mode_access() {
    let model = Arc::new(RecordingModel::default());
    let mut config = Config {
        active_mode: Some("plan".into()),
        features: Some(Features {
            tools: ["ExtraTools".into()].into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    config
        .policy
        .tools
        .denied_tools
        .insert("denied_read".into());
    let mut bundle = builder(config, &model)
        .extra_tools([
            tool("allowed_read", true),
            tool("denied_read", true),
            tool("write", false),
        ])
        .build(&context())
        .await
        .unwrap();
    assert!(
        bundle
            .agent()
            .instructions
            .contains("Tool access: read-only\nAvailable tools include: allowed_read.")
    );
    assert!(!bundle.agent().instructions.contains("denied_read"));
    assert!(!bundle.agent().instructions.contains("write"));
    bundle.close().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn workspace_prompt_rejects_non_utf8_paths_without_lossy_instructions() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let config = Config {
        work_dir: OsString::from_vec(b"/workspace/\xff".to_vec()).into(),
        ..Default::default()
    };
    let error = builder(config, &Arc::new(RecordingModel::default()))
        .build(&context())
        .await
        .err()
        .unwrap();
    assert_eq!(error.info.category, ErrorCategory::InvalidInput);
    assert_eq!(error.info.message, "workspace prompt path must be UTF-8");
}

#[tokio::test]
async fn builder_maps_run_instruction_sections_without_changing_agent_text() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/run-instructions/observations.json"
    ))
    .unwrap();
    for case in fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["builder"] == true)
    {
        let model = Arc::new(RecordingModel::default());
        let config = Config {
            instructions: "base".into(),
            work_dir: "".into(),
            features: Some(Features::default()),
            feature_summary: case["feature_summary"].as_str().unwrap().into(),
            mode_directive_text: case["mode_directive_text"].as_str().unwrap().into(),
            final_check_instructions: case["final_check_instructions"].as_str().unwrap().into(),
            ..Default::default()
        };
        let mut bundle = builder(config, &model).build(&context()).await.unwrap();
        assert_eq!(bundle.agent().instructions, "base");
        if case["streaming"].as_bool().unwrap() {
            bundle
                .stream(context(), vec![], Arc::new(TestHost))
                .finish()
                .await
                .unwrap();
        } else {
            bundle
                .run(context(), vec![], Arc::new(TestHost))
                .await
                .unwrap();
        }
        assert_eq!(
            model.requests.lock().unwrap()[0].instructions,
            case["instructions"].as_str().unwrap()
        );
        bundle.close().await.unwrap();
    }
}

struct BlankCarry;
impl adk::runtime::CompactionCarryForward for BlankCarry {
    fn context<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async { Ok(String::new()) })
    }
}

#[tokio::test]
async fn working_state_fallback_matches_sdk_and_is_only_injected_after_compaction() {
    use adk::runtime::compaction::LocalCompactionPolicy;
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/run-instructions/observations.json"
    ))
    .unwrap();
    for case in fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["builder"] == true)
    {
        let model = Arc::new(RecordingModel::default());
        let mut bundle = builder(
            Config {
                features: Some(Features {
                    compaction: true,
                    ..Default::default()
                }),
                ..Default::default()
            },
            &model,
        )
        .runner_config(RunnerConfig {
            working_state_context: case["working_state_text"].as_str().unwrap().into(),
            compaction_carry_forward: Some(Arc::new(BlankCarry)),
            local_compaction: LocalCompactionPolicy {
                use_llm_summary: false,
                trigger_tokens: 30_000,
                target_tokens: 20_000,
                preserve_recent_items: 1,
                preserve_initial_user_messages: 1,
                ..Default::default()
            },
            ..Default::default()
        })
        .build(&context())
        .await
        .unwrap();
        let message = |text: String| RunItem::Message {
            message: Message {
                role: Role::User,
                content: vec![Content::Text { text }],
            },
        };
        let history = (0..20)
            .map(|_| message("old conversation ".repeat(5000)))
            .collect();
        if case["streaming"] == true {
            bundle
                .stream(context(), vec![message("hello".into())], Arc::new(TestHost))
                .finish()
                .await
                .unwrap();
            bundle
                .stream(context(), history, Arc::new(TestHost))
                .finish()
                .await
                .unwrap();
        } else {
            bundle
                .run(context(), vec![message("hello".into())], Arc::new(TestHost))
                .await
                .unwrap();
            bundle
                .run(context(), history, Arc::new(TestHost))
                .await
                .unwrap();
        }
        {
            let requests = model.requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            let expected = case["working_state_context"].as_str().unwrap().trim();
            assert_eq!(requests[0].input, vec![message("hello".into())]);
            assert!(!requests[0].instructions.contains(expected));
            let carry = requests[1]
                .input
                .iter()
                .filter_map(|item| match item {
                    RunItem::Message { message } => {
                        message.content.iter().find_map(|content| match content {
                            Content::Text { text }
                                if text.starts_with("[COMPACTION CARRY-FORWARD]") =>
                            {
                                Some(text)
                            }
                            _ => None,
                        })
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(carry.len(), 1, "{case}");
            assert!(carry[0].ends_with(expected), "{case}: {:?}", carry[0]);
        }
        bundle.close().await.unwrap();
    }
}

#[cfg(feature = "observability")]
#[tokio::test]
async fn builder_compaction_matches_pinned_requests_for_model_defaults_and_host_policy() {
    use adk::runtime::compaction::LocalCompactionPolicy;
    use sha2::{Digest, Sha256};
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/run-instructions/observations.json"
    ))
    .unwrap();
    for case in fixture["compaction_cases"].as_array().unwrap() {
        let model = Arc::new(RecordingModel::default());
        let mut runner = RunnerConfig {
            working_state_context: case["working_state_text"].as_str().unwrap().into(),
            local_compaction: LocalCompactionPolicy {
                use_llm_summary: false,
                ..Default::default()
            },
            ..Default::default()
        };
        if case["blank_carry"] == true {
            runner.compaction_carry_forward = Some(Arc::new(BlankCarry));
        }
        if case["custom"] == true {
            runner.local_compaction = LocalCompactionPolicy {
                enabled: true,
                use_llm_summary: false,
                trigger_tokens: 90000,
                target_tokens: 40000,
                preserve_recent_items: 3,
                preserve_initial_user_messages: 1,
                summary_bullet_limit: 7,
            };
        }
        let provider = match case["provider"].as_str().unwrap() {
            "" => None,
            name => Some(name.parse::<Kind>().unwrap()),
        };
        let mode_name = case["mode_name"].as_str().unwrap();
        let explicit_policy = case["explicit_policy"].as_bool().map(|enabled| {
            if case["default_policy"] == true {
                LocalCompactionPolicy {
                    use_llm_summary: false,
                    ..Default::default()
                }
            } else {
                LocalCompactionPolicy {
                    enabled,
                    use_llm_summary: false,
                    trigger_tokens: 90000,
                    target_tokens: 40000,
                    preserve_recent_items: 3,
                    preserve_initial_user_messages: 1,
                    summary_bullet_limit: 7,
                }
            }
        });
        let mut bundle = Builder::new(Config {
            provider,
            local_compaction: explicit_policy,
            mode_snapshot: (!mode_name.is_empty()).then(|| ModeSpec {
                name: mode_name.into(),
                display_name: case["mode_display_name"].as_str().unwrap().into(),
                tool_access: "full".into(),
                ..Default::default()
            }),
            model: case["model"].as_str().unwrap().into(),
            instructions: "base".into(),
            work_dir: "".into(),
            features: Some(Features {
                compaction: case["feature_enabled"].as_bool().unwrap_or(true),
                ..Default::default()
            }),
            ..Default::default()
        })
        .model(
            provider.unwrap_or(Kind::OpenAi).name(),
            provider.unwrap_or(Kind::OpenAi),
            model.clone(),
        )
        .unwrap()
        .runner_config(runner)
        .build(&context())
        .await
        .unwrap();
        let history = (0..100)
            .map(|i| RunItem::Message {
                message: Message {
                    role: Role::User,
                    content: vec![Content::Text {
                        text: format!(
                            "message {i:03}: {}",
                            "old conversation ".repeat(if i >= 80 {
                                2
                            } else {
                                case["input_repeat"].as_u64().unwrap_or(2000) as usize
                            })
                        ),
                    }],
                },
            })
            .collect();
        if case["streaming"] == true {
            bundle
                .stream(context(), history, Arc::new(TestHost))
                .finish()
                .await
                .unwrap();
        } else {
            bundle
                .run(context(), history, Arc::new(TestHost))
                .await
                .unwrap();
        }
        {
            let requests = model.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            let hashes = requests[0]
                .input
                .iter()
                .map(|item| {
                    let RunItem::Message { message } = item else {
                        panic!("unexpected nonmessage")
                    };
                    let [Content::Text { text }] = message.content.as_slice() else {
                        panic!("unexpected content")
                    };
                    format!("{:x}", Sha256::digest(text.as_bytes()))
                })
                .collect::<Vec<_>>();
            assert_eq!(json!(hashes), case["text_sha256"], "{case}");
        }
        bundle.close().await.unwrap();
    }
}

#[tokio::test]
async fn builder_defaults_to_llm_summary_and_respects_explicit_false() {
    use adk::runtime::compaction::{LocalCompactionPolicy, extract_summary};
    for enabled in [true, false] {
        let model = Arc::new(RecordingModel::default());
        let mut bundle = builder(
            Config {
                model: "gpt-6-mini".into(),
                local_compaction: (!enabled).then(|| LocalCompactionPolicy {
                    use_llm_summary: false,
                    ..Default::default()
                }),
                features: Some(Features {
                    compaction: true,
                    ..Default::default()
                }),
                ..Default::default()
            },
            &model,
        )
        .build(&context())
        .await
        .unwrap();
        let message = |role, text| RunItem::Message {
            message: Message {
                role,
                content: vec![Content::Text { text }],
            },
        };
        bundle
            .run(
                context(),
                vec![
                    message(Role::User, "task".into()),
                    message(Role::Assistant, "old content ".repeat(80_000)),
                    message(Role::Assistant, "latest".into()),
                ],
                Arc::new(TestHost),
            )
            .await
            .unwrap();
        {
            let requests = model.requests.lock().unwrap();
            assert_eq!(requests.len(), if enabled { 2 } else { 1 });
            let summary = extract_summary(&requests.last().unwrap().input);
            assert!(!summary.is_empty());
            assert_eq!(summary.ends_with("offline"), enabled);
            if enabled {
                assert!(requests[0].instructions.starts_with("You are compacting"));
                assert_eq!(requests[0].settings["max_tokens"], 2048);
            }
        }
        bundle.close().await.unwrap();
    }
}
