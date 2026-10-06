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
                ..Default::default()
            }),
            legacy_tools: adk::tools::LegacyFeatures {
                enable_subagents: true,
                ..Default::default()
            },
            ..Default::default()
        };
        config.policy.tools.access = access;
        let model = Arc::new(RecordingModel::default());
        let mut bundle = builder(config, &model)
            .extra_tools(names.iter().map(|n| tool(n, true)))
            .build(&context())
            .await
            .unwrap();
        let block = case["workspace"].as_str().unwrap();
        assert_eq!(
            bundle.agent().instructions,
            if block.is_empty() {
                "host instructions".to_owned()
            } else {
                format!("host instructions\n\n{block}")
            }
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
