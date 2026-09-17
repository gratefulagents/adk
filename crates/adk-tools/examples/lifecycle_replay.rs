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
    name: String,
    arguments: Value,
    #[serde(default)]
    full_access: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    for line in io::stdin().lock().lines() {
        let request: Request = serde_json::from_str(&line.unwrap()).unwrap();
        #[cfg(target_os = "linux")]
        let skills = adk_tools::skills::tools(
            serde_json::from_str(include_str!("../../../fixtures/tools/skill-catalog.json"))
                .unwrap(),
            request.work_dir.clone(),
            Default::default(),
        );
        #[cfg(not(target_os = "linux"))]
        let skills: Vec<Arc<dyn adk_core::Tool>> = Vec::new();
        let registry = Registry::build(
            &Config {
                access: if request.full_access {
                    adk_core::AccessMode::FullAccess
                } else {
                    adk_core::AccessMode::WorkspaceWrite
                },
                features: Features::Strict(
                    ["Move", "Delete", "Write", "Edit", "ExtraTools"]
                        .map(String::from)
                        .into(),
                ),
                ..Default::default()
            },
            skills,
        )
        .unwrap();
        let context = ToolContext {
            operation: Context {
                run_id: "lifecycle-replay".into(),
                cancellation: Arc::new(adk_runtime::CancellationToken::new()),
                deadline: None,
            },
            work_dir: request.work_dir,
            policy: Default::default(),
            idempotency_key: None,
        };
        let output = registry
            .get(&request.name)
            .unwrap()
            .execute(
                &context,
                ToolCall {
                    id: "fixture".into(),
                    name: request.name,
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
