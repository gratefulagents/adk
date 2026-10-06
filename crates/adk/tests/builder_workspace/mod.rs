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
            local_compaction: LocalCompactionPolicy {
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
