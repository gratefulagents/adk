use adk_core::{Context, ToolCall, ToolContext};
use adk_tools::{Config, Features, Registry};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
    sync::Arc,
};

#[derive(Deserialize)]
struct Request {
    work_dir: PathBuf,
    arguments: Value,
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let registry = Registry::build(
        &Config {
            access: adk_core::AccessMode::WorkspaceWrite,
            features: Features::Strict(["ApplyPatch".into()].into()),
            ..Default::default()
        },
        [],
    )
    .unwrap();
    for line in io::stdin().lock().lines() {
        let request: Request = serde_json::from_str(&line.unwrap()).unwrap();
        let context = ToolContext {
            operation: Context {
                run_id: "patch-replay".into(),
                cancellation: Arc::new(adk_runtime::CancellationToken::new()),
                deadline: None,
            },
            work_dir: request.work_dir,
            policy: Default::default(),
            idempotency_key: None,
        };
        let output = registry
            .get("ApplyPatch")
            .unwrap()
            .execute(
                &context,
                ToolCall {
                    id: "fixture".into(),
                    name: "ApplyPatch".into(),
                    arguments: request.arguments,
                },
            )
            .await;
        let value = match output {
            Ok(output) => serde_json::to_value(output).unwrap(),
            Err(error) => json!({"error":error.to_string()}),
        };
        println!("{}", serde_json::to_string(&value).unwrap());
        io::stdout().flush().unwrap();
    }
}
