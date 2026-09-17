use adk_core::*;
use adk_project_state::*;
use serde_json::{Value, json};
use std::sync::Arc;

struct NeverCancel;
impl Cancellation for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
async fn invoke(tools: &[Arc<dyn Tool>], name: &str, arguments: Value, error: bool) -> String {
    let context = ToolContext {
        operation: Context {
            run_id: "tools-test".into(),
            cancellation: Arc::new(NeverCancel),
            deadline: None,
        },
        work_dir: Default::default(),
        policy: Default::default(),
        idempotency_key: None,
    };
    let output = tools
        .iter()
        .find(|t| t.definition().name == name)
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
    assert_eq!(output.is_error, error, "{name}: {output:?}");
    assert!(!output.should_pause);
    match &output.content[..] {
        [Content::Text { text }] => text.clone(),
        _ => panic!("non-text result"),
    }
}
async fn json_call(tools: &[Arc<dyn Tool>], name: &str, args: Value) -> Value {
    serde_json::from_str(&invoke(tools, name, args, false).await).unwrap()
}

#[tokio::test]
async fn baseline_state_tool_contracts_on_filesystem_and_sqlite() {
    for sqlite in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let open = || {
            let store = StoreOptions {
                project_id: "tool-contract".into(),
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
        let tools = tools::tools(open(), " agent ");
        assert_eq!(tools.len(), 15);
        for tool in &tools {
            let def = tool.definition();
            assert!(!def.requires_approval);
            assert_eq!(
                def.read_only,
                matches!(
                    def.name.as_str(),
                    "task_ready"
                        | "task_show"
                        | "memory_recall"
                        | "memory_list"
                        | "memory_stats"
                        | "prime_context"
                )
            );
        }
        let a = json_call(&tools, "task_create", json!({"title":"blocker"})).await;
        assert_eq!(a["priority"], 2);
        let b = json_call(
            &tools,
            "task_create",
            json!({"title":"dependent","priority":0}),
        )
        .await;
        assert_eq!(b["priority"], 0);
        json_call(
            &tools,
            "task_link",
            json!({"id":b["id"],"depends_on":a["id"]}),
        )
        .await;
        assert_eq!(
            json_call(&tools, "task_ready", json!({}))
                .await
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let claimed = json_call(&tools, "task_claim", json!({"id":a["id"]})).await;
        assert_eq!(claimed["assignee"], "agent");
        let comment = json_call(&tools, "task_comment", json!({"id":a["id"],"body":"note"})).await;
        assert_eq!(comment["actor"], "agent");
        json_call(
            &tools,
            "task_update",
            json!({"id":a["id"],"labels":["label"]}),
        )
        .await;
        let cleared = json_call(&tools, "task_update", json!({"id":a["id"],"labels":[]})).await;
        assert!(cleared.get("labels").is_none());
        json_call(&tools, "task_close", json!({"id":a["id"],"reason":"done"})).await;
        assert_eq!(
            json_call(&tools, "task_ready", json!({})).await[0]["id"],
            b["id"]
        );
        json_call(
            &tools,
            "task_link",
            json!({"id":b["id"],"depends_on":a["id"],"action":"remove"}),
        )
        .await;
        let shown = json_call(&tools, "task_show", json!({"id":a["id"]})).await;
        assert_eq!(shown["status"], "closed");
        let mem = json_call(&tools,"memory_remember",json!({"content":"durable engineering preference","kind":"semantic","scope":"user","tags":["style"],"task_ids":[b["id"]]})).await;
        let updated = json_call(
            &tools,
            "memory_update",
            json!({"id":mem["id"],"content":"compact engineering preference","tags":[]}),
        )
        .await;
        assert_eq!(updated["scope"], "user");
        assert_eq!(updated["task_ids"], mem["task_ids"]);
        assert!(updated.get("tags").is_none());
        assert_eq!(
            json_call(&tools, "memory_list", json!({"kinds":["semantic"]}))
                .await
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            json_call(&tools, "memory_recall", json!({"query":"engineering"})).await[0]["id"],
            mem["id"]
        );
        assert_eq!(
            json_call(&tools, "memory_stats", json!({})).await,
            json!({"total":1,"by_kind":{"semantic":1},"by_scope":{"user":1},"by_tag":{}})
        );
        assert!(
            invoke(
                &tools,
                "prime_context",
                json!({"active_task_id":b["id"]}),
                false
            )
            .await
            .contains("dependent")
        );
        drop(tools);
        let tools = tools::tools(open(), "agent");
        assert_eq!(
            json_call(&tools, "memory_list", json!({})).await[0]["content"],
            updated["content"]
        );
        json_call(&tools, "memory_delete", json!({"id":mem["id"]})).await;
        assert_eq!(
            json_call(&tools, "memory_stats", json!({})).await["total"],
            0
        );
        invoke(
            &tools,
            "memory_update",
            json!({"id":mem["id"],"content":"must not resurrect"}),
            true,
        )
        .await;
        invoke(&tools, "task_show", json!({"id":3}), true).await;
        invoke(&tools, "task_create", json!({"title":""}), true).await;
    }
}

fn normalize(value: &mut Value, ids: &std::collections::BTreeMap<String, String>) {
    match value {
        Value::Object(map) => {
            for value in map.values_mut() {
                normalize(value, ids);
            }
        }
        Value::Array(array) => {
            for value in array {
                normalize(value, ids);
            }
        }
        Value::String(text) => {
            if let Some(id) = ids.get(text) {
                *text = id.clone();
            } else if chrono::DateTime::parse_from_rfc3339(text).is_ok() {
                *text = "<time>".into();
            } else if text.starts_with("comment_") {
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
async fn actual_go_tool_outputs_match_both_stores() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/project-state/tools.json")).unwrap();
    for sqlite in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let opts = StoreOptions {
            project_id: "tool-contract".into(),
            ..Default::default()
        };
        let store = if sqlite {
            ProjectStore::sqlite(SQLiteOptions {
                path: dir.path().join("state.db"),
                store: opts,
                ..Default::default()
            })
            .unwrap()
        } else {
            ProjectStore::filesystem(FilesystemOptions {
                state_dir: dir.path().join("state"),
                store: opts,
                ..Default::default()
            })
            .unwrap()
        };
        let tools = tools::tools(Arc::new(store), "agent");
        assert_eq!(
            serde_json::to_value(
                tools
                    .iter()
                    .map(|tool| tool.definition())
                    .collect::<Vec<_>>()
            )
            .unwrap(),
            fixture["definitions"]
        );
        let covered: std::collections::BTreeSet<_> = fixture["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step["name"].as_str().unwrap())
            .collect();
        assert_eq!(covered.len(), 15);
        assert!(
            fixture["steps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|step| step["is_error"] == true)
        );
        let mut ids = std::collections::BTreeMap::new();
        let mut reverse = std::collections::BTreeMap::new();
        for step in fixture["steps"].as_array().unwrap() {
            let mut input = step["input"].clone();
            normalize(&mut input, &ids);
            let is_error = step["is_error"].as_bool().unwrap();
            let output = invoke(&tools, step["name"].as_str().unwrap(), input, is_error).await;
            let mut result = if step["format"] == "text" {
                Value::String(output)
            } else {
                serde_json::from_str(&output).unwrap()
            };
            if is_error
                && step["output"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("Invalid input:"))
            {
                assert!(
                    result.as_str().unwrap().starts_with("Invalid input:"),
                    "{result}"
                );
                continue;
            }
            if let Some(save) = step["save"].as_str() {
                let id = result["id"].as_str().unwrap().to_owned();
                ids.insert(save.to_owned(), id.clone());
                reverse.insert(id, save.to_owned());
            }
            normalize(&mut result, &reverse);
            assert_eq!(result, step["output"], "{} sqlite={sqlite}", step["name"]);
        }
    }
}
