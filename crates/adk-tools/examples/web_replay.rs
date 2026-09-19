use adk_core::{Context, ToolCall, ToolContext};
use adk_tools::{Config, Features, Registry};
use serde_json::Value;
use std::{
    io::{self, BufRead, Write},
    sync::Arc,
};
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let registry = Registry::build(
        &Config {
            features: Features::Strict(["WebFetch".into()].into()),
            allow_private_network_urls: true,
            ..Default::default()
        },
        [],
    )
    .unwrap();
    for line in io::stdin().lock().lines() {
        let arguments: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let context = ToolContext {
            operation: Context {
                run_id: "web-replay".into(),
                cancellation: Arc::new(adk_runtime::CancellationToken::new()),
                deadline: None,
            },
            work_dir: Default::default(),
            policy: Default::default(),
            idempotency_key: None,
        };
        let output = registry
            .get("WebFetch")
            .unwrap()
            .execute(
                &context,
                ToolCall {
                    id: "fixture".into(),
                    name: "WebFetch".into(),
                    arguments,
                },
            )
            .await
            .unwrap();
        println!("{}", serde_json::to_string(&output).unwrap());
        io::stdout().flush().unwrap();
    }
}
