use adk_core::*;
use adk_project_state::memory::InMemoryStore;
use adk_tools::{Config, Features, Registry, memory};
use serde_json::{Value, json};
use std::sync::Arc;

async fn call(registry: &Registry, input: Value) -> ToolOutput {
    registry
        .get("Memory")
        .unwrap()
        .execute(
            &ToolContext {
                operation: Context {
                    run_id: "caller".into(),
                    cancellation: Arc::new(adk_runtime::CancellationToken::new()),
                    deadline: None,
                },
                work_dir: Default::default(),
                policy: ToolPolicy {
                    access: AccessMode::WorkspaceWrite,
                    ..Default::default()
                },
                idempotency_key: None,
            },
            ToolCall {
                id: "call".into(),
                name: "Memory".into(),
                arguments: input,
            },
        )
        .await
        .unwrap()
}
fn text(output: &ToolOutput) -> &str {
    assert!(!output.should_pause);
    match &output.content[..] {
        [Content::Text { text }] => text,
        _ => panic!("text expected"),
    }
}
fn config() -> Config {
    Config {
        features: Features::Strict(["ExtraTools".to_string()].into()),
        access: AccessMode::WorkspaceWrite,
        ..Default::default()
    }
}

#[tokio::test]
async fn host_namespace_identity_and_all_memory_actions() {
    let store = Arc::new(InMemoryStore::new());
    let registry = Registry::build(
        &config(),
        [memory::tool(
            store.clone(),
            "trusted",
            "source",
            "https://example.org/repo",
        )],
    )
    .unwrap();
    assert_eq!(
        text(&call(&registry, json!({"action":"list"})).await),
        "No memories found."
    );
    let output = call(&registry,json!({"action":"store","content":"tool contract","tags":["registry"],"namespace":"spoof","source_run":"spoof","repo":"spoof"})).await;
    assert!(!output.is_error);
    let saved: Value = serde_json::from_str(text(&output)).unwrap();
    assert_eq!(saved["namespace"], "trusted");
    assert_eq!(saved["source_run"], "source");
    assert_eq!(
        saved["metadata"],
        json!({"repo":"https://example.org/repo"})
    );
    for action in ["search", "list"] {
        let output = call(
            &registry,
            json!({"action":action,"content":"contract","tags":["registry"],"limit":1}),
        )
        .await;
        assert!(!output.is_error);
        let found: Value = serde_json::from_str(text(&output)).unwrap();
        assert_eq!(found.as_array().unwrap().len(), 1);
        assert_eq!(found[0]["id"], saved["id"]);
    }
    assert_eq!(
        text(&call(&registry, json!({"action":"search","content":"unrelated"})).await),
        "No matching memories found."
    );
    let other = Registry::build(
        &config(),
        [memory::tool(store.clone(), "other", "source", "")],
    )
    .unwrap();
    assert_eq!(
        text(&call(&other, json!({"action":"list"})).await),
        "No memories found."
    );
    assert!(
        call(&other, json!({"action":"delete","id":saved["id"]}))
            .await
            .is_error
    );
    assert_eq!(
        text(&call(&registry, json!({"action":"delete","id":saved["id"]})).await),
        format!("Memory {} deleted.", saved["id"].as_str().unwrap())
    );
    assert_eq!(
        text(&call(&registry, json!({"action":"list"})).await),
        "No memories found."
    );
    let config = Config {
        access: AccessMode::ReadOnly,
        ..config()
    };
    let registry =
        Registry::build(&config, [memory::tool(store, "trusted", "source", "")]).unwrap();
    assert!(registry.get("Memory").is_none());
}

#[tokio::test]
async fn malformed_requests_and_store_errors_do_not_succeed() {
    let registry = Registry::build(
        &config(),
        [memory::tool(
            Arc::new(InMemoryStore::new()),
            "ns",
            "source",
            "",
        )],
    )
    .unwrap();
    for input in [
        json!(42),
        json!({}),
        json!({"action":"invalid"}),
        json!({"action":"store"}),
        json!({"action":"search"}),
        json!({"action":"delete"}),
        json!({"action":"delete","id":"bad"}),
        json!({"action":"delete","id":"00000000-0000-0000-0000-000000000000"}),
    ] {
        let output = call(&registry, input.clone()).await;
        assert!(output.is_error, "{input}: {output:?}");
        assert!(!output.should_pause);
    }
    let registry = Registry::build(
        &config(),
        [memory::tool(
            Arc::new(InMemoryStore::new()),
            "",
            "source",
            "",
        )],
    )
    .unwrap();
    for action in ["store", "search", "list"] {
        let output = call(&registry, json!({"action":action,"content":"required"})).await;
        assert!(output.is_error);
        assert!(text(&output).starts_with("Failed to "));
    }
}
