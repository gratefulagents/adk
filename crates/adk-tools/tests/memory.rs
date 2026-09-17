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
    assert!(registry.get("Memory").is_some());
    assert!(
        !registry
            .prepare(ToolPolicy::default())
            .tools
            .iter()
            .any(|tool| tool.definition().name == "Memory")
    );
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

fn normalize_contract(value: &mut Value, ids: &std::collections::BTreeMap<String, String>) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(key.as_str(), "created_at" | "updated_at" | "closed_at")
                    && value.is_string()
                {
                    *value = json!("<time>");
                } else if key == "similarity" {
                    *value = json!(value.as_f64().unwrap());
                } else {
                    normalize_contract(value, ids);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                normalize_contract(value, ids);
            }
        }
        Value::String(text) => {
            if text.starts_with("comment_") {
                *text = "<comment>".into();
            } else {
                for (id, alias) in ids {
                    *text = text.replace(id, alias);
                }
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn pinned_go_nullable_fields_tags_and_padded_uuid_delete() {
    let registry = Registry::build(
        &config(),
        [memory::tool(
            Arc::new(InMemoryStore::new()),
            "trusted",
            "source",
            "",
        )],
    )
    .unwrap();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/tools/state-memory-plan-expected.json"
    ))
    .unwrap();
    let mut ids = std::collections::BTreeMap::new();
    let mut reverse = std::collections::BTreeMap::new();
    for step in fixture["memory"].as_array().unwrap() {
        let mut input = step["input"].clone();
        normalize_contract(&mut input, &ids);
        let output = call(&registry, input).await;
        assert_eq!(output.is_error, step["is_error"], "{step:?}: {output:?}");
        let mut result = if step["format"] == "text" {
            json!(text(&output))
        } else {
            serde_json::from_str(text(&output)).unwrap()
        };
        if let Some(alias) = step["save"].as_str() {
            let id = result["id"].as_str().unwrap().to_owned();
            ids.insert(alias.to_owned(), id.clone());
            reverse.insert(id, alias.to_owned());
        }
        normalize_contract(&mut result, &reverse);
        let mut expected = step["output"].clone();
        normalize_contract(&mut expected, &Default::default());
        assert_eq!(result, expected, "{step:?}");
    }
}

#[derive(Default)]
struct RecordingStore {
    inner: InMemoryStore,
    calls: std::sync::Mutex<Vec<Value>>,
}
impl adk_project_state::memory::Store for RecordingStore {
    fn store(
        &self,
        namespace: &str,
        content: &str,
        tags: &[String],
        source_run: &str,
        metadata: Value,
    ) -> adk_project_state::Result<adk_project_state::memory::Memory> {
        self.inner
            .store(namespace, content, tags, source_run, metadata)
    }
    fn search(
        &self,
        namespace: &str,
        query: &str,
        tags: &[String],
        limit: i32,
    ) -> adk_project_state::Result<Vec<adk_project_state::memory::Memory>> {
        self.calls
            .lock()
            .unwrap()
            .push(json!(["search", namespace, query, tags, limit]));
        self.inner.search(namespace, query, tags, limit)
    }
    fn list(
        &self,
        namespace: &str,
        tags: &[String],
        limit: i32,
    ) -> adk_project_state::Result<Vec<adk_project_state::memory::Memory>> {
        self.calls
            .lock()
            .unwrap()
            .push(json!(["list", namespace, tags, limit]));
        self.inner.list(namespace, tags, limit)
    }
    fn delete(&self, namespace: &str, id: uuid::Uuid) -> adk_project_state::Result<()> {
        self.inner.delete(namespace, id)
    }
}

#[tokio::test]
async fn recording_store_proves_forwarding_binding_limits_defaults_and_tag_exclusion() {
    use adk_project_state::memory::Store;
    let store = Arc::new(RecordingStore::default());
    let registry = Registry::build(
        &config(),
        [memory::tool(store.clone(), "trusted", "source", "")],
    )
    .unwrap();
    let mut ids = Vec::new();
    for i in 0..55 {
        ids.push(
            store
                .store(
                    "trusted",
                    &format!("needle {i}"),
                    &["keep".into()],
                    "source",
                    json!({}),
                )
                .unwrap()
                .id
                .to_string(),
        );
    }
    store
        .store(
            "trusted",
            "needle excluded",
            &["skip".into()],
            "source",
            json!({}),
        )
        .unwrap();
    store
        .store(
            "other",
            "needle other namespace",
            &["keep".into()],
            "source",
            json!({}),
        )
        .unwrap();
    store
        .store(
            "trusted",
            "unrelated",
            &["keep".into()],
            "source",
            json!({}),
        )
        .unwrap();
    for action in ["search", "list"] {
        for limit in [json!(1), json!(2), json!(0), json!(-1), Value::Null] {
            let output = call(&registry, json!({"action":action,"content":"needle","tags":["keep"],"limit":limit,"namespace":"other"})).await;
            assert!(!output.is_error);
            let found: Vec<Value> = serde_json::from_str(text(&output)).unwrap();
            let forwarded = limit.as_i64().unwrap_or(0);
            let expected = if forwarded > 0 {
                forwarded as usize
            } else if action == "search" {
                10
            } else {
                50
            };
            assert_eq!(found.len(), expected);
            assert!(
                found
                    .iter()
                    .all(|m| m["namespace"] == "trusted" && m["tags"] == json!(["keep"]))
            );
            if action == "search" {
                assert_eq!(
                    found
                        .iter()
                        .map(|m| m["id"].as_str().unwrap())
                        .collect::<Vec<_>>(),
                    ids.iter()
                        .rev()
                        .take(expected)
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                );
            } else {
                assert_eq!(found[0]["content"], "unrelated");
            }
            let recorded = store.calls.lock().unwrap().pop().unwrap();
            assert_eq!(
                recorded,
                if action == "search" {
                    json!([action, "trusted", "needle", ["keep"], forwarded])
                } else {
                    json!([action, "trusted", ["keep"], forwarded])
                }
            );
        }
    }
}

#[test]
fn namespace_memory_requires_explicit_host_injection() {
    let registry = Registry::build(&config(), []).unwrap();
    assert!(registry.get("Memory").is_none());
}
