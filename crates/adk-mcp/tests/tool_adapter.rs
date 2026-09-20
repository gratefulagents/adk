use adk_core::{Cancellation, Content, Context, ToolCall, ToolContext, ToolDefinition, ToolPolicy};
use adk_mcp::{
    BoxFuture, Error,
    tools::{ToolManager, build_tools},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
struct NeverCancelled;
impl Cancellation for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
struct Manager(AtomicUsize);
impl ToolManager for Manager {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            name: "mcp__test__lookup".into(),
            description: "untrusted test".into(),
            input_schema: json!({"type":"object"}).try_into().unwrap(),
            read_only: true,
            requires_approval: false,
        }]
    }
    fn call<'a>(&'a self, name: &'a str, _: Value) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(async move {
            assert_eq!(name, "mcp__test__lookup");
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"content":[{"type":"text","text":"peer error"}],"isError":true}))
        })
    }
    fn list_resources<'a>(&'a self, _: Option<&'a str>) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(async { Ok(json!([])) })
    }
    fn read_resource<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(async { Ok(json!({"contents":[]})) })
    }
    fn has_resources(&self) -> bool {
        true
    }
}
#[tokio::test]
async fn native_tools_preserve_results_validate_arguments_and_expose_resources() {
    let temp = tempfile::tempdir().unwrap();
    let context = ToolContext {
        operation: Context {
            run_id: "test".into(),
            cancellation: Arc::new(NeverCancelled),
            deadline: None,
        },
        work_dir: temp.path().to_owned(),
        policy: ToolPolicy::default(),
        idempotency_key: None,
    };
    let manager = Arc::new(Manager(AtomicUsize::new(0)));
    let tools = build_tools(manager.clone());
    assert_eq!(tools.len(), 3);
    let dynamic = &tools[0];
    let result = dynamic
        .execute(
            &context,
            ToolCall {
                id: "one".into(),
                name: dynamic.definition().name.clone(),
                arguments: json!({}),
            },
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert_eq!(
        result.content,
        vec![Content::Text {
            text: "peer error".into()
        }]
    );
    let result = dynamic
        .execute(
            &context,
            ToolCall {
                id: "two".into(),
                name: dynamic.definition().name.clone(),
                arguments: json!([]),
            },
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert_eq!(manager.0.load(Ordering::SeqCst), 1);
    for tool in &tools[1..] {
        assert!(tool.definition().read_only);
        let args = if tool.definition().name == "ReadMcpResourceTool" {
            json!({"server":"test","uri":"test://resource"})
        } else {
            json!({})
        };
        let result = tool
            .execute(
                &context,
                ToolCall {
                    id: "r".into(),
                    name: tool.definition().name.clone(),
                    arguments: args,
                },
            )
            .await
            .unwrap();
        assert!(!result.is_error);
    }
}
