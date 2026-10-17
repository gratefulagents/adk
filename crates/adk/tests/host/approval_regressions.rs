use super::*;
use std::num::NonZeroU32;

struct RecordingTool {
    definition: ToolDefinition,
    executed: Arc<Mutex<Vec<String>>>,
    output: &'static str,
}

impl Tool for RecordingTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async {
            self.executed
                .lock()
                .unwrap()
                .push(self.definition.name.clone());
            Ok(ToolOutput {
                content: vec![Content::Text {
                    text: self.output.into(),
                }],
                is_error: false,
                should_pause: false,
            })
        })
    }
}

struct BlockingTool(ToolDefinition);

impl Tool for BlockingTool {
    fn definition(&self) -> &ToolDefinition {
        &self.0
    }

    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(std::future::pending())
    }
}

fn scripted_approval(
    fixture: &Arc<Fixture>,
    calls: &[(&str, &str, &str)],
    output: &'static str,
    final_text: &str,
    executed: &Arc<Mutex<Vec<String>>>,
) -> ChatLoopOptions {
    let mut agent = AgentConfig::new("loop", ModelBinding::complete("demo", fixture.clone()));
    let mut items = vec![];
    for &(id, name, input) in calls {
        agent.tools.push(Arc::new(RecordingTool {
            definition: ToolDefinition {
                name: name.into(),
                description: "mutates".into(),
                input_schema: serde_json::from_value(json!({"type":"object"})).unwrap(),
                read_only: false,
                requires_approval: false,
            },
            executed: executed.clone(),
            output,
        }));
        items.push(RunItem::ToolCall {
            call: ToolCall {
                id: id.into(),
                name: name.into(),
                arguments: serde_json::from_str(input).unwrap(),
                raw_arguments: Some(input.into()),
            },
        });
    }
    let mut first = response(items, false);
    first.end_turn = None;
    let mut second = response(vec![item(final_text, Role::Assistant)], true);
    second.end_turn = None;
    *fixture.responses.lock().unwrap() = VecDeque::from([Ok(first), Ok(second)]);
    let mut options = ChatLoopOptions::new(agent);
    options.max_turns = NonZeroU32::new(3);
    options.approval_required = Some(true);
    options.approval_gate = Some(fixture.clone());
    options
}

#[tokio::test]
async fn resolves_tool_approval_and_resumes_with_stable_cache_identity() {
    let fixture = Arc::new(Fixture::default());
    let executed = Arc::new(Mutex::new(vec![]));
    let options = scripted_approval(
        &fixture,
        &[("mutate_call", "mutate", r#"{"value":"x"}"#)],
        "mutated",
        "approved done",
        &executed,
    );
    let result = ChatLoop::new(options).run(context()).await.unwrap();
    assert_eq!(*executed.lock().unwrap(), ["mutate"]);
    assert_eq!(result.result.final_text(), "approved done");
    let requests = fixture.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let first = requests[0].settings["prompt_cache_key"].as_str().unwrap();
    assert_eq!(first.len(), 64);
    assert_eq!(requests[1].settings["prompt_cache_key"], first);
}

#[tokio::test]
async fn resolves_all_parallel_tool_approvals() {
    let fixture = Arc::new(Fixture::default());
    let executed = Arc::new(Mutex::new(vec![]));
    let options = scripted_approval(
        &fixture,
        &[("c1", "mutate_a", "{}"), ("c2", "mutate_b", "{}")],
        "ok",
        "both done",
        &executed,
    );
    let result = ChatLoop::new(options).run(context()).await.unwrap();
    assert_eq!(result.result.final_text(), "both done");
    assert_eq!(fixture.gate_requests.lock().unwrap().len(), 2);
    assert_eq!(executed.lock().unwrap().len(), 2);
    for expected in ["c1", "c2"] {
        assert!(result.result.new_items.iter().any(|item| matches!(item,
            RunItem::ToolResult { call_id, .. } if call_id == expected
        )));
    }
}

#[tokio::test]
async fn approved_tool_uses_runner_timeout_policy() {
    let fixture = Arc::new(Fixture::default());
    let executed = Arc::new(Mutex::new(vec![]));
    let mut options = scripted_approval(
        &fixture,
        &[("slow_call", "slow_mutate", r#"{"value":"x"}"#)],
        "",
        "resumed after timeout",
        &executed,
    );
    options.agent.tools[0] = Arc::new(BlockingTool(options.agent.tools[0].definition().clone()));
    options.policy.tools.timeout = Some(Duration::from_secs(1));
    let start = tokio::time::Instant::now();
    let result = ChatLoop::new(options).run(context()).await.unwrap();
    assert!(start.elapsed() <= Duration::from_secs(3));
    assert_eq!(result.result.final_text(), "resumed after timeout");
    assert!(result.result.new_items.iter().any(|item| matches!(item,
        RunItem::ToolResult { output, .. } if output.content.iter().any(|content|
            matches!(content, Content::Text { text } if text.contains("timed out")))
    )));
}

#[tokio::test]
async fn cache_namespace_is_per_invocation_unless_explicitly_shared() {
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/host-loop/sdk-chatloop.json"
    ))
    .unwrap();
    let expected = &oracle["cases"][0]["runs"][0]["calls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|call| call["op"] == "model.response")
        .unwrap()["args"]["prompt_cache_key"];
    for namespace in [None, Some(" \t "), Some(" host-reference-fixed\n")] {
        let fixture = Arc::new(Fixture::default());
        let mut options = ChatLoopOptions::new(AgentConfig::new(
            "loop",
            ModelBinding::complete("demo", fixture.clone()),
        ));
        options.runner_config.prompt_cache_namespace = namespace.map(str::to_owned);
        options.runner_config.prompt_cache_key = Some(" \t ".into());
        let mut chat = ChatLoop::new(options);
        chat.run(context()).await.unwrap();
        chat.run(context()).await.unwrap();
        let requests = fixture.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let first = requests[0].settings["prompt_cache_key"].as_str().unwrap();
        let second = requests[1].settings["prompt_cache_key"].as_str().unwrap();
        assert_eq!(first.len(), 64);
        assert_eq!(second.len(), 64);
        if namespace.is_some_and(|namespace| !namespace.trim().is_empty()) {
            assert_eq!(first, expected.as_str().unwrap());
            assert_eq!(first, second);
        } else {
            assert_ne!(first, second);
        }
    }
}
