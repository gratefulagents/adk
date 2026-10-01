//! Offline embedding: the application owns input, persistence and loop lifetime.
use adk::{
    core::*,
    host::{ChatLoop, ChatLoopOptions, Cursor, RunBatch, SessionStore, UserMessage, WorkingState},
    runtime::{AgentConfig, CancellationToken, ModelBinding},
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Store(Mutex<Vec<RunBatch>>);
impl SessionStore for Store {
    fn load_messages<'a>(
        &'a self,
        _: &'a Context,
        cursor: &'a Cursor,
        _: usize,
    ) -> BoxFuture<'a, Result<(Vec<UserMessage>, Cursor), Error>> {
        Box::pin(async move {
            if cursor.message_id != 0 {
                return Ok((vec![], cursor.clone()));
            }
            Ok((
                vec![UserMessage {
                    id: 1,
                    content: "Hello from the host".into(),
                    ..Default::default()
                }],
                Cursor {
                    message_id: 1,
                    token: "page-one".into(),
                },
            ))
        })
    }
    fn append_run_items<'a>(
        &'a self,
        _: &'a Context,
        batch: &'a RunBatch,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(batch.clone());
            Ok(())
        })
    }
    fn working_state<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<WorkingState, Error>> {
        Box::pin(async { Ok(WorkingState::default()) })
    }
}
struct OfflineModel;
impl Model for OfflineModel {
    fn provider(&self) -> &str {
        "offline"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            assert_eq!(request.input.len(), 1);
            Ok(ModelResponse {
                items: vec![RunItem::Message {
                    message: Message {
                        role: Role::Assistant,
                        content: vec![Content::Text {
                            text: "Hello from the embedded ADK".into(),
                        }],
                    },
                }],
                usage: Usage::default(),
                end_turn: Some(true),
                response_id: None,
                raw: None,
                snapshot_raw: None,
                snapshot_projection: None,
                metadata: Default::default(),
            })
        })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(Store::default());
    let agent = AgentConfig::new(
        "embedded",
        ModelBinding::complete("offline", Arc::new(OfflineModel)),
    );
    let mut options = ChatLoopOptions::new(agent);
    options.session_store = Some(store.clone());
    let mut session = ChatLoop::new(options);
    let result = session
        .run(Context {
            run_id: "standalone-host-example".into(),
            cancellation: Arc::new(CancellationToken::new()),
            deadline: None,
        })
        .await?;
    assert_eq!(result.result.status, RunStatus::Completed);
    assert_eq!(session.cursor().token, "page-one");
    assert_eq!(store.0.lock().unwrap().len(), 1);
    assert_eq!(store.0.lock().unwrap()[0].items, result.result.new_items);
    session.close().await?;
    println!("Offline host session: input consumed, response persisted, owner closed");
    Ok(())
}
