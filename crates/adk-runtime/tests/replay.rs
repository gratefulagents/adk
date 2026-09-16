use std::{
    collections::VecDeque,
    num::NonZeroU32,
    sync::{Arc, Mutex},
};

use adk_core::*;
use adk_runtime::CancellationToken;
use serde_json::{Value, json};

fn text(content: &[Content]) -> String {
    content
        .iter()
        .map(|part| match part {
            Content::Text { text } => text.as_str(),
            _ => panic!("unexpected non-text content"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalized(items: &[RunItem]) -> Vec<Value> {
    items.iter().map(|item| match item {
        RunItem::Message { message } => json!({"type":"message", "text":text(&message.content)}),
        RunItem::ToolCall { call } => json!({"type":"tool_call", "id":call.id, "name":call.name, "arguments":call.arguments}),
        RunItem::ToolResult { call_id, output } => {
            assert!(!output.should_pause);
            let mut value = json!({"type":"tool_result", "id":call_id, "content":text(&output.content)});
            if output.is_error { value["is_error"] = json!(true); }
            value
        }
        _ => panic!("unexpected handoff"),
    }).collect()
}

fn input_items(items: &Value, role: Role) -> Vec<RunItem> {
    items
        .as_array()
        .unwrap()
        .iter()
        .map(|item| match item["type"].as_str().unwrap() {
            "message" => RunItem::Message {
                message: Message {
                    role,
                    content: vec![Content::Text {
                        text: item["text"].as_str().unwrap().into(),
                    }],
                },
            },
            "tool_call" => RunItem::ToolCall {
                call: ToolCall {
                    id: item["id"].as_str().unwrap().into(),
                    name: item["name"].as_str().unwrap().into(),
                    arguments: item["arguments"].clone(),
                },
            },
            other => panic!("unsupported script item {other}"),
        })
        .collect()
}

struct ScriptModel {
    responses: Mutex<VecDeque<Value>>,
    requests: Mutex<Vec<Value>>,
    streaming: bool,
}

impl ScriptModel {
    fn respond(&self, request: ModelRequest) -> Result<(ModelResponse, Vec<String>), Error> {
        self.requests.lock().unwrap().push(json!({
            "model":request.model, "instructions":request.instructions,
            "input":normalized(&request.input), "tools":request.tools.iter().map(|t| &t.name).collect::<Vec<_>>()
        }));
        let script = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("model script exhausted");
        if let Some(error) = script["error"].as_str() {
            return Err(Error::new(ErrorCategory::Provider, error));
        }
        let response = ModelResponse {
            items: input_items(&script["items"], Role::Assistant),
            usage: Usage {
                input_tokens: script["input_tokens"].as_u64().unwrap(),
                output_tokens: script["output_tokens"].as_u64().unwrap(),
                ..Usage::default()
            },
            end_turn: script["end_turn"].as_bool(),
            response_id: None,
            metadata: Default::default(),
        };
        let deltas = script["deltas"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().into())
            .collect();
        Ok((response, deltas))
    }
}

impl Model for ScriptModel {
    fn provider(&self) -> &str {
        "replay"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            assert!(!self.streaming, "streaming path silently used completion");
            Ok(self.respond(request)?.0)
        })
    }
}

struct ScriptStream(VecDeque<ModelEvent>);
impl ModelStream for ScriptStream {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async move { Ok(self.0.pop_front()) })
    }
}
impl StreamingModel for ScriptModel {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            assert!(self.streaming);
            let (response, deltas) = self.respond(request)?;
            let mut events: VecDeque<_> = deltas
                .into_iter()
                .map(|delta| ModelEvent::TextDelta { delta })
                .collect();
            events.push_back(ModelEvent::Complete { response });
            Ok(Box::new(ScriptStream(events)) as Box<dyn ModelStream>)
        })
    }
}

struct Echo {
    definition: ToolDefinition,
    dispatch: Mutex<Vec<Value>>,
}
impl Tool for Echo {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            let output = format!("echo: {}", call.arguments["text"].as_str().unwrap());
            self.dispatch.lock().unwrap().push(
                json!({"name":self.definition.name,"arguments":call.arguments,"output":output}),
            );
            Ok(ToolOutput {
                content: vec![Content::Text { text: output }],
                is_error: false,
                should_pause: false,
            })
        })
    }
}

#[derive(Default)]
struct RecordingHost(Mutex<Vec<RunEvent>>);
impl Host for RecordingHost {
    fn emit<'a>(&'a self, _: &'a Context, event: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(event);
            Ok(())
        })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { Ok(ApprovalDecision::Defer) })
    }
}

fn normalized_events(events: &[RunEvent]) -> Vec<Value> {
    let mut out = vec![];
    for event in events {
        match event {
            RunEvent::Model {
                event: ModelEvent::TextDelta { delta },
            } => out.push(json!({"type":"text_delta","delta":delta})),
            RunEvent::Model {
                event: ModelEvent::Complete { response },
            } => {
                for item in normalized(&response.items) {
                    out.push(json!({"type":"item","item":item}));
                }
            }
            RunEvent::ToolFinished { call_id, output } => {
                let item = RunItem::ToolResult {
                    call_id: call_id.clone(),
                    output: output.clone(),
                };
                out.push(json!({"type":"item","item":normalized(&[item])[0]}));
            }
            RunEvent::ApprovalRequired { .. }
            | RunEvent::Started { .. }
            | RunEvent::ToolStarted { .. }
            | RunEvent::Finished { .. }
            | RunEvent::Failed { .. } => {}
            other => panic!("unmapped event: {other:?}"),
        }
    }
    out
}

async fn replay(script: &Value) -> Value {
    use adk_runtime::{AgentConfig, ModelBinding, Runner, RunnerConfig, output::OutputPolicy};
    let streaming = script["streaming"].as_bool().unwrap();
    let model = Arc::new(ScriptModel {
        responses: Mutex::new(
            script["responses"]
                .as_array()
                .unwrap()
                .iter()
                .cloned()
                .collect(),
        ),
        requests: Mutex::new(vec![]),
        streaming,
    });
    let tool = Arc::new(Echo {
        definition:ToolDefinition {
            name:"echo".into(),description:"Echo text".into(),
            input_schema:serde_json::from_value(json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]})).unwrap(),
            read_only:true, requires_approval:false,
        }, dispatch:Mutex::new(vec![]),
    });
    let binding = if streaming {
        ModelBinding::streaming("replay-model", model.clone())
    } else {
        ModelBinding::complete("replay-model", model.clone())
    };
    let mut agent = AgentConfig::new("replay-agent", binding);
    agent.instructions = "Follow the replay script.".into();
    agent.tools.push(tool.clone());
    if script["approvals"].as_bool() == Some(true) {
        let mut definition = tool.definition.clone();
        definition.name = "approval".into();
        definition.requires_approval = true;
        agent.tools.push(Arc::new(Echo {
            definition,
            dispatch: Mutex::new(vec![]),
        }));
    }
    if let Some(names) = script["fallbacks"].as_array() {
        agent.fallbacks = names
            .iter()
            .map(|name| {
                if streaming {
                    ModelBinding::streaming(name.as_str().unwrap(), model.clone())
                } else {
                    ModelBinding::complete(name.as_str().unwrap(), model.clone())
                }
            })
            .collect();
    }
    if !script["schema"].is_null() {
        agent.output_schema = Some(script["schema"].clone().try_into().unwrap());
    }
    let runner = Runner::new(
        agent,
        RunnerConfig {
            output: OutputPolicy {
                untrusted: false,
                ..OutputPolicy::default()
            },
            model_idle_timeout: None,
            ..RunnerConfig::default()
        },
    )
    .unwrap();
    let host = Arc::new(RecordingHost::default());
    let context = Context {
        run_id: script["name"].as_str().unwrap().into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    };
    let request = RunRequest {
        input: input_items(&script["input"], Role::User),
        policy: RunPolicy {
            max_turns: NonZeroU32::new(script["max_turns"].as_u64().unwrap().try_into().unwrap())
                .unwrap(),
            tools: ToolPolicy::default(),
            tool_use: ToolUseBehavior::Continue,
        },
    };
    let outcome = if streaming {
        let mut stream = runner.stream(context, request, host.clone());
        let mut pulled = vec![];
        while let Some(event) = stream.next().await {
            pulled.push(event);
        }
        assert_eq!(
            &pulled,
            &*host.0.lock().unwrap(),
            "pull and host event channels diverged"
        );
        stream.finish().await
    } else {
        runner.run(context, request, host.clone()).await
    };
    let (result, error) = match outcome {
        Ok(outcome) => {
            assert_eq!(
                outcome.continuation.is_some(),
                script["approvals"].as_bool() == Some(true)
            );
            assert!(outcome.spills.is_empty());
            (outcome.result, Value::Null)
        }
        Err(failure) => {
            assert_eq!(failure.error.info.category, ErrorCategory::MaxTurns);
            (
                *failure
                    .partial
                    .expect("budget failure must retain partial history"),
                json!("max_turns"),
            )
        }
    };
    assert_eq!(
        result.pending_approvals.is_empty(),
        script["approvals"].as_bool() != Some(true)
    );
    assert!(
        model.responses.lock().unwrap().is_empty(),
        "runner stopped before consuming the script"
    );
    let events = host.0.lock().unwrap();
    assert!(matches!(events.first(),Some(RunEvent::Started { agent }) if agent == "replay-agent"));
    match events.last().unwrap() {
        RunEvent::Finished { result: emitted } => assert_eq!(emitted, &result),
        RunEvent::Failed { error: emitted } => {
            assert_eq!(emitted.category, ErrorCategory::MaxTurns)
        }
        other => panic!("missing terminal event: {other:?}"),
    }
    let mut observation = json!({
        "requests":*model.requests.lock().unwrap(), "dispatch":*tool.dispatch.lock().unwrap(),
        "events":if streaming { normalized_events(&events) } else { vec![] },
        "outcome":{
            "error":error,"status":result.status,"final_output":result.final_output,
            "history":normalized(&result.history),"new_items":normalized(&result.new_items),
            "response_count":result.responses.len(),"last_agent":result.last_agent,
            "input_tokens":result.usage.input_tokens,"output_tokens":result.usage.output_tokens,
        }
    });
    if script["approvals"].as_bool() == Some(true) {
        observation["pending"] = json!(normalized(
            &result
                .pending_approvals
                .iter()
                .map(|request| RunItem::ToolCall {
                    call: request.call.clone()
                })
                .collect::<Vec<_>>()
        ));
    }
    observation
}

fn fixtures() -> Value {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/runner.json")).unwrap();
    assert_eq!(fixture["schema_version"], 1);
    assert_eq!(
        fixture["sdk_revision"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    fixture
}

#[tokio::test(flavor = "current_thread")]
async fn actual_rust_runner_matches_actual_go_runner() {
    let fixture = fixtures();
    let inputs: Value =
        serde_json::from_str(include_str!("../../../fixtures/runner_inputs.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 22);
    assert_eq!(
        cases
            .iter()
            .map(|case| case["input"].clone())
            .collect::<Vec<_>>(),
        *inputs.as_array().unwrap()
    );
    for case in cases {
        let observed =
            tokio::time::timeout(std::time::Duration::from_secs(5), replay(&case["input"]))
                .await
                .unwrap();
        assert_eq!(
            observed, case["expected"],
            "Go/Rust runner divergence: {}",
            case["input"]["name"]
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn replay_observations_depend_on_execution_not_goldens() {
    let fixture = fixtures();
    let case = &fixture["cases"][1];
    let mut changed = case["input"].clone();
    changed["responses"][0]["items"][0]["arguments"]["text"] = json!("mutated argument");
    changed["responses"][0]["items"][0]["id"] = json!("mutated-call-id");
    let observed = replay(&changed).await;
    for field in ["requests", "dispatch", "outcome"] {
        assert_ne!(
            observed[field], case["expected"][field],
            "mutation was hidden: {field}"
        );
    }
    assert_eq!(observed["dispatch"][0]["output"], "echo: mutated argument");
    assert_eq!(observed["outcome"]["new_items"][1]["id"], "mutated-call-id");
    let case = &fixture["cases"][4];
    let mut changed = case["input"].clone();
    changed["responses"][0]["deltas"][0] = json!("changed delta");
    let observed = replay(&changed).await;
    assert_ne!(observed["events"], case["expected"]["events"]);
    assert_eq!(observed["outcome"], case["expected"]["outcome"]);
}
