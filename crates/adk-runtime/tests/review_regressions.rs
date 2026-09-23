//! Regressions found during independent review of the execution state machine.
use adk_core::*;
use adk_runtime::*;
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

fn message(content: Content) -> RunItem {
    RunItem::Message {
        message: Message {
            role: Role::Assistant,
            content: vec![content],
        },
    }
}
fn text(value: &str) -> RunItem {
    message(Content::Text { text: value.into() })
}
fn response(items: Vec<RunItem>) -> ModelResponse {
    ModelResponse {
        raw: None,
        items,
        usage: Usage::default(),
        end_turn: Some(true),
        response_id: None,
        metadata: Default::default(),
    }
}
fn context() -> Context {
    Context {
        run_id: "review-regressions".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn policy() -> RunPolicy {
    RunPolicy {
        max_turns: 8.try_into().unwrap(),
        tools: ToolPolicy::default(),
        tool_use: ToolUseBehavior::Continue,
    }
}
fn request() -> RunRequest {
    RunRequest {
        input_provenance: Vec::new(),
        input: vec![],
        policy: policy(),
    }
}
struct HostSink;
impl Host for HostSink {
    fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { Ok(ApprovalDecision::Defer) })
    }
}
struct Events(VecDeque<Result<ModelEvent, Error>>);
impl ModelStream for Events {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async { self.0.pop_front().transpose() })
    }
}
struct Script {
    responses: Mutex<VecDeque<Result<ModelResponse, Error>>>,
    events: Mutex<VecDeque<Result<ModelEvent, Error>>>,
}
impl Script {
    fn new(
        responses: Vec<Result<ModelResponse, Error>>,
        events: Vec<Result<ModelEvent, Error>>,
    ) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            events: Mutex::new(events.into()),
        }
    }
}
impl Model for Script {
    fn provider(&self) -> &str {
        "review-fake"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected model dispatch")
        })
    }
}
impl StreamingModel for Script {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async {
            Ok(
                Box::new(Events(std::mem::take(&mut *self.events.lock().unwrap())))
                    as Box<dyn ModelStream>,
            )
        })
    }
}

#[tokio::test]
async fn final_nonempty_message_not_commentary_is_validated_in_both_modes() {
    for streamed in [false, true] {
        let result = response(vec![
            text("Checking the result…"),
            text("{\"ok\":true}"),
            text(""),
        ]);
        let fake = Arc::new(Script::new(
            vec![Ok(result.clone())],
            vec![Ok(ModelEvent::Complete { response: result })],
        ));
        let mut agent = AgentConfig::new("review", ModelBinding::streaming("fake", fake));
        agent.output_schema = Some(json!({"type":"object", "required":["ok"], "properties":{"ok":{"const":true}}, "additionalProperties":false}).try_into().unwrap());
        let runner = Runner::new(agent, RunnerConfig::default()).unwrap();
        let outcome = if streamed {
            runner
                .stream(context(), request(), Arc::new(HostSink))
                .finish()
                .await
        } else {
            runner.run(context(), request(), Arc::new(HostSink)).await
        }
        .unwrap();
        assert_eq!(outcome.result.final_output, Some(json!({"ok":true})));
        assert_eq!(
            outcome.result.new_items.len(),
            3,
            "history retains commentary and empty message"
        );
    }
}

#[tokio::test]
async fn failed_stream_retains_answer_after_reasoning_or_completed_text() {
    for previous in [
        message(Content::Reasoning {
            text: "thought".into(),
            signature: None,
        }),
        text("previous answer"),
    ] {
        let fake = Arc::new(Script::new(
            vec![],
            vec![
                Ok(ModelEvent::ItemDone {
                    item: previous.clone(),
                }),
                Ok(ModelEvent::TextDelta {
                    delta: "partial answer".into(),
                }),
                Err(Error::new(ErrorCategory::Provider, "stream disconnected")),
            ],
        ));
        let runner = Runner::new(
            AgentConfig::new("review", ModelBinding::streaming("fake", fake)),
            RunnerConfig::default(),
        )
        .unwrap();
        let error = runner
            .stream(context(), request(), Arc::new(HostSink))
            .finish()
            .await
            .err()
            .expect("stream must fail");
        let partial = error.partial.unwrap();
        assert_eq!(partial.history, vec![previous, text("partial answer")]);
        assert_eq!(partial.history, partial.new_items);
        assert_eq!(partial.status, RunStatus::Incomplete);
    }
}

#[tokio::test]
async fn failed_stream_does_not_duplicate_deltas_already_committed_as_item() {
    let fake = Arc::new(Script::new(
        vec![],
        vec![
            Ok(ModelEvent::TextDelta {
                delta: "completed".into(),
            }),
            Ok(ModelEvent::ItemDone {
                item: text("completed"),
            }),
            Ok(ModelEvent::TextDelta {
                delta: "unfinished".into(),
            }),
            Err(Error::new(ErrorCategory::Provider, "stream disconnected")),
        ],
    ));
    let runner = Runner::new(
        AgentConfig::new("review", ModelBinding::streaming("fake", fake)),
        RunnerConfig::default(),
    )
    .unwrap();
    let error = runner
        .stream(context(), request(), Arc::new(HostSink))
        .finish()
        .await
        .err()
        .unwrap();
    assert_eq!(
        error.partial.unwrap().history,
        vec![text("completed"), text("unfinished")]
    );
}

struct CountTool {
    definition: ToolDefinition,
    count: Arc<AtomicUsize>,
}
impl Tool for CountTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async {
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(ToolOutput {
                content: vec![Content::Text {
                    text: "completed once".into(),
                }],
                is_error: false,
                should_pause: false,
            })
        })
    }
}

#[tokio::test]
async fn conversation_can_adopt_failed_resume_and_continue_without_repeated_effect() {
    let count = Arc::new(AtomicUsize::new(0));
    let fake = Arc::new(Script::new(
        vec![
            Ok(response(vec![RunItem::ToolCall {
                call: ToolCall {
                    id: "approved-call".into(),
                    name: "count".into(),
                    arguments: json!({}),
                },
            }])),
            Err(Error::new(ErrorCategory::Provider, "after completed tool")),
            Ok(response(vec![text("recovered")])),
        ],
        vec![],
    ));
    let mut agent = AgentConfig::new("review", ModelBinding::complete("fake", fake));
    agent.tools.push(Arc::new(CountTool {
        definition: ToolDefinition {
            name: "count".into(),
            description: "test".into(),
            input_schema: json!({"type":"object"}).try_into().unwrap(),
            read_only: true,
            requires_approval: true,
        },
        count: count.clone(),
    }));
    let runner = Runner::new(agent, RunnerConfig::default()).unwrap();
    let mut conversation = Conversation::default();
    let mut paused = conversation
        .run(&runner, context(), vec![], policy(), Arc::new(HostSink))
        .await
        .unwrap();
    assert_eq!(paused.result.status, RunStatus::Paused);
    let error = paused
        .continuation
        .take()
        .unwrap()
        .resume(Some(ApprovalDecision::Approve))
        .await
        .err()
        .expect("model fails after tool completes");
    assert_eq!(count.load(Ordering::SeqCst), 1);
    conversation.accept_error(&error);
    assert!(conversation.history.iter().any(
        |item| matches!(item, RunItem::ToolResult { call_id, .. } if call_id == "approved-call")
    ));
    let recovered = conversation
        .run(&runner, context(), vec![], policy(), Arc::new(HostSink))
        .await
        .unwrap();
    assert_eq!(recovered.result.final_output, Some(json!("recovered")));
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
