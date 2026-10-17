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

struct SecretGuard(&'static str);

impl adk::runtime::Guardrail for SecretGuard {
    fn name(&self) -> &str {
        self.0
    }

    fn check<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
        input: adk::runtime::GuardrailInput<'a>,
    ) -> BoxFuture<'a, Result<Option<adk::runtime::GuardrailResult>, Error>> {
        Box::pin(async move {
            let (tripwire_triggered, output) = match input {
                adk::runtime::GuardrailInput::ToolInput(call) => (
                    call.argument_text() == r#"{"value":"secret"}"#,
                    json!("blocked"),
                ),
                adk::runtime::GuardrailInput::ToolOutput { output, .. } => (
                    output.content.iter().any(|content| {
                        matches!(content,
                        Content::Text { text } if text.contains("secret"))
                    }),
                    serde_json::Value::Null,
                ),
                _ => panic!("unexpected guardrail input"),
            };
            Ok(Some(adk::runtime::GuardrailResult {
                tripwire_triggered,
                output,
                replacement_content: None,
            }))
        })
    }
}

struct RuleSource(Vec<GuardrailRule>);

impl ConfigSource for RuleSource {
    fn permission_mode<'a>(
        &'a self,
        _: &'a Context,
    ) -> BoxFuture<'a, Result<PermissionMode, Error>> {
        Box::pin(async { Ok(PermissionMode::DangerFullAccess) })
    }

    fn guardrail_rules<'a>(
        &'a self,
        _: &'a Context,
    ) -> BoxFuture<'a, Result<Vec<GuardrailRule>, Error>> {
        Box::pin(async { Ok(self.0.clone()) })
    }
}

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
async fn runner_and_config_guardrails_survive_host_approval_boundaries() {
    for from_config in [false, true] {
        for output in [false, true] {
            let fixture = Arc::new(Fixture::default());
            let executed = Arc::new(Mutex::new(vec![]));
            let mut options = scripted_approval(
                &fixture,
                &[(
                    "mutate_call",
                    "mutate",
                    if output {
                        r#"{"value":"x"}"#
                    } else {
                        r#"{"value":"secret"}"#
                    },
                )],
                if output { "secret output" } else { "mutated" },
                "guardrail handled",
                &executed,
            );
            options.max_turns = NonZeroU32::new(2);
            if from_config {
                options.approval_required = Some(false);
                options.approval_gate = None;
                options.config_source = Some(Arc::new(RuleSource(vec![GuardrailRule {
                    name: if output {
                        "block-secret-output"
                    } else {
                        "block-secret"
                    }
                    .into(),
                    rule_type: if output { "tool-output" } else { "tool-input" }.into(),
                    regex: "secret".into(),
                    tool_pattern: "mutate".into(),
                    action: "block".into(),
                    message: if output {
                        "blocked output by config rule"
                    } else {
                        "blocked by config rule"
                    }
                    .into(),
                }])));
            } else if output {
                options
                    .runner_config
                    .tool_output_guardrails
                    .push(Arc::new(SecretGuard("block-output")));
            } else {
                options
                    .runner_config
                    .tool_input_guardrails
                    .push(Arc::new(SecretGuard("block-secret")));
            }
            let result = ChatLoop::new(options).run(context()).await.unwrap();
            assert_eq!(executed.lock().unwrap().len(), usize::from(output));
            assert_eq!(result.result.final_text(), "guardrail handled");
            assert!(result.result.new_items.len() >= 3);
            let needle = if output {
                "tool output guardrail"
            } else {
                "tool input guardrail"
            };
            assert!(
                result.result.new_items.iter().any(|item| matches!(item,
                    RunItem::ToolResult { output, .. } if output.content.iter().any(|content|
                        matches!(content, Content::Text { text } if text.contains(needle)))
                )),
                "from_config={from_config}, output={output}"
            );
        }
    }
}

#[tokio::test]
async fn invalid_config_rules_fail_closed_with_compile_context_before_model() {
    let fixture = Arc::new(Fixture::default());
    fixture.responses.lock().unwrap().push_back(Ok(response(
        vec![item("should not run", Role::Assistant)],
        true,
    )));
    let mut options = ChatLoopOptions::new(AgentConfig::new(
        "loop",
        ModelBinding::complete("demo", fixture.clone()),
    ));
    options.max_turns = NonZeroU32::new(1);
    options.config_source = Some(Arc::new(RuleSource(vec![GuardrailRule {
        name: "bad-regex".into(),
        rule_type: "tool-input".into(),
        regex: "[".into(),
        action: "block".into(),
        ..Default::default()
    }])));
    let error = ChatLoop::new(options).run(context()).await.err().unwrap();
    assert!(error.to_string().contains("compile guardrail rules"));
    assert!(fixture.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn input_rules_match_original_argument_text_in_pinned_host_runs() {
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/host-loop/sdk-chatloop.json"
    ))
    .unwrap();
    let cases: Vec<_> = oracle["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| {
            case["name"]
                .as_str()
                .unwrap()
                .starts_with("raw-input-rule-")
        })
        .collect();
    assert_eq!(cases.len(), 4);
    for case in cases {
        let fixture = Arc::new(Fixture::default());
        let executed = Arc::new(Mutex::new(vec![]));
        let raw = case["spec"]["RawInput"].as_str().unwrap();
        fixture.rules.lock().unwrap().push(GuardrailRule {
            name: "raw".into(),
            regex: case["spec"]["Rules"][0]["Regex"].as_str().unwrap().into(),
            action: "block".into(),
            rule_type: "tool-input".into(),
            tool_pattern: "write".into(),
            ..Default::default()
        });
        let mut options = scripted_approval(
            &fixture,
            &[("raw-call", "write", raw)],
            "secret output",
            "done",
            &executed,
        );
        options.approval_required = Some(false);
        options.config_source = Some(fixture.clone());
        let result = ChatLoop::new(options).run(context()).await.unwrap();
        let expected = &case["runs"][0];
        assert!(expected["error"].is_null());
        assert_eq!(
            result.result.final_text(),
            expected["result"]["final_text"].as_str().unwrap()
        );
        assert_eq!(
            executed.lock().unwrap().len(),
            expected["calls"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|call| call["op"] == "tool.execute")
                .count(),
            "{}",
            case["name"]
        );
        assert_eq!(result.result.guardrails.len(), 1);
        assert_eq!(
            result.result.guardrails[0].tripwire_triggered,
            expected["result"]["tool_input_guardrails"][0]["TripwireTriggered"]
                .as_bool()
                .unwrap(),
            "{}",
            case["name"]
        );
    }
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
