//! Run with: cargo run -p adk --features runtime --example runner
//! No provider credentials or platform services are used.
use std::{collections::VecDeque, sync::Arc};

use adk::core::*;
use adk::runtime::*;

struct Echo;

fn response() -> ModelResponse {
    ModelResponse {
        raw: None,
        items: vec![RunItem::Message {
            message: Message {
                role: Role::Assistant,
                content: vec![Content::Text {
                    text: "Hello from the standalone Rust runner!".into(),
                }],
            },
        }],
        usage: Usage::default(),
        end_turn: Some(true),
        response_id: None,
        metadata: Default::default(),
    }
}

impl Model for Echo {
    fn provider(&self) -> &str {
        "local-example"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async { Ok(response()) })
    }
}

struct Events(VecDeque<ModelEvent>);
impl ModelStream for Events {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async { Ok(self.0.pop_front()) })
    }
}
impl StreamingModel for Echo {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async {
            Ok(Box::new(Events(VecDeque::from([
                ModelEvent::TextDelta {
                    delta: "Hello from the standalone Rust runner!".into(),
                },
                ModelEvent::Complete {
                    response: response(),
                },
            ]))) as Box<dyn ModelStream>)
        })
    }
}

struct Console;
impl Host for Console {
    fn emit<'a>(&'a self, _: &'a Context, event: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            if let RunEvent::Model {
                event: ModelEvent::TextDelta { delta },
            } = event
            {
                println!("delta: {delta}");
            }
            Ok(())
        })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        // An unattended example must never silently approve side effects.
        Box::pin(async { Ok(ApprovalDecision::Defer) })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runner = Runner::new(
        AgentConfig::new("example", ModelBinding::streaming("echo", Arc::new(Echo))),
        RunnerConfig::default(),
    )?;
    let context = Context {
        run_id: "standalone-example".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    };
    let request = RunRequest {
        input_provenance: Vec::new(),
        input: vec![RunItem::Message {
            message: Message {
                role: Role::User,
                content: vec![Content::Text {
                    text: "Say hello".into(),
                }],
            },
        }],
        policy: RunPolicy {
            max_turns: 3.try_into()?,
            tools: ToolPolicy::default(),
            tool_use: ToolUseBehavior::Continue,
        },
    };
    let normal = runner
        .run(context.clone(), request.clone(), Arc::new(Console))
        .await?;
    println!("normal: {:?}", normal.result.final_output);

    let mut stream = runner.stream(context, request, Arc::new(Console));
    while let Some(event) = stream.next().await {
        if let RunEvent::Finished { result } = event {
            println!("stream complete: {:?}", result.final_output);
        }
    }
    let streamed = stream.finish().await?;
    assert_eq!(normal.result.final_output, streamed.result.final_output);
    Ok(())
}
