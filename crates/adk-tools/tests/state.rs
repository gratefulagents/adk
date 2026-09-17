use adk_core::*;
use adk_project_state::*;
use adk_tools::{Config, Features, Registry, project_state_tools};
use serde_json::{Value, json};
use std::sync::Arc;

fn config(access: AccessMode) -> Config {
    Config {
        features: Features::Strict(
            [
                "ProjectState.TaskTools",
                "ProjectState.MemoryTools",
                "ProjectState.PrimeTool",
            ]
            .map(String::from)
            .into(),
        ),
        access,
        ..Default::default()
    }
}
async fn invoke(registry: &Registry, name: &str, arguments: Value) -> String {
    let context = ToolContext {
        operation: Context {
            run_id: "registry-state".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: Default::default(),
        policy: ToolPolicy {
            access: AccessMode::WorkspaceWrite,
            ..Default::default()
        },
        idempotency_key: None,
    };
    let output = registry
        .get(name)
        .unwrap()
        .execute(
            &context,
            ToolCall {
                id: "call".into(),
                name: name.into(),
                arguments,
            },
        )
        .await
        .unwrap();
    assert!(!output.is_error, "{name}: {output:?}");
    assert!(!output.should_pause);
    match &output.content[..] {
        [Content::Text { text }] => text.clone(),
        _ => panic!("expected text"),
    }
}
async fn call(registry: &Registry, name: &str, args: Value) -> Value {
    serde_json::from_str(&invoke(registry, name, args).await).unwrap()
}

#[tokio::test]
async fn all_fifteen_tools_integrate_with_durable_filesystem_and_sqlite_stores() {
    for sqlite in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let open = || {
            let store = StoreOptions {
                project_id: "registry-state".into(),
                ..Default::default()
            };
            Arc::new(if sqlite {
                ProjectStore::sqlite(SQLiteOptions {
                    path: dir.path().join("state.db"),
                    store,
                    ..Default::default()
                })
                .unwrap()
            } else {
                ProjectStore::filesystem(FilesystemOptions {
                    state_dir: dir.path().join("state"),
                    store,
                    ..Default::default()
                })
                .unwrap()
            })
        };
        let build = |access| {
            Registry::build(&config(access), project_state_tools(open(), " actor ")).unwrap()
        };
        let registry = build(AccessMode::WorkspaceWrite);
        assert_eq!(registry.names().count(), 15);
        let a = call(&registry, "task_create", json!({"title":"blocker"})).await;
        let b = call(&registry, "task_create", json!({"title":"dependent"})).await;
        call(
            &registry,
            "task_link",
            json!({"id":b["id"],"depends_on":a["id"]}),
        )
        .await;
        assert_eq!(
            call(&registry, "task_ready", json!({}))
                .await
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            call(&registry, "task_claim", json!({"id":a["id"]})).await["assignee"],
            "actor"
        );
        assert_eq!(
            call(
                &registry,
                "task_comment",
                json!({"id":a["id"],"body":"note"})
            )
            .await["actor"],
            "actor"
        );
        call(
            &registry,
            "task_update",
            json!({"id":a["id"],"labels":["checked"]}),
        )
        .await;
        call(&registry, "task_close", json!({"id":a["id"]})).await;
        assert_eq!(
            call(&registry, "task_show", json!({"id":a["id"]})).await["status"],
            "closed"
        );
        let memory = call(
            &registry,
            "memory_remember",
            json!({"content":"registry evidence","kind":"semantic"}),
        )
        .await;
        call(
            &registry,
            "memory_update",
            json!({"id":memory["id"],"content":"durable registry evidence"}),
        )
        .await;
        assert_eq!(
            call(&registry, "memory_list", json!({}))
                .await
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            call(&registry, "memory_recall", json!({"query":"registry"})).await[0]["id"],
            memory["id"]
        );
        assert_eq!(call(&registry, "memory_stats", json!({})).await["total"], 1);
        assert!(
            invoke(
                &registry,
                "prime_context",
                json!({"active_task_id":b["id"]})
            )
            .await
            .contains("dependent")
        );
        drop(registry);
        let registry = build(AccessMode::ReadOnly);
        assert_eq!(
            registry.names().collect::<Vec<_>>(),
            [
                "memory_list",
                "memory_recall",
                "memory_stats",
                "prime_context",
                "task_ready",
                "task_show"
            ]
        );
        assert_eq!(
            call(&registry, "memory_list", json!({})).await[0]["id"],
            memory["id"]
        );
        assert_eq!(
            call(&registry, "task_show", json!({"id":a["id"]})).await["status"],
            "closed"
        );
        drop(registry);
        let registry = build(AccessMode::WorkspaceWrite);
        call(&registry, "memory_delete", json!({"id":memory["id"]})).await;
        assert_eq!(call(&registry, "memory_stats", json!({})).await["total"], 0);
    }
}
