use adk_core::*;
use adk_runtime::{subagent::*, *};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Echo(Mutex<Vec<ChildInvocation>>);
impl ChildExecutor for Echo {
    fn execute<'a>(
        &'a self,
        invocation: ChildInvocation,
        _: ChildControl,
    ) -> BoxFuture<'a, Result<ChildOutcome, Error>> {
        Box::pin(async move {
            let text = serde_json::to_string(&invocation.request.input).unwrap();
            self.0.lock().unwrap().push(invocation);
            Ok(ChildOutcome::completed(text))
        })
    }
}
fn context() -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "tools".into(),
            cancellation: Arc::new(CancellationToken::new()),
            deadline: None,
        },
        policy: ToolPolicy {
            access: AccessMode::FullAccess,
            ..Default::default()
        },
        work_dir: ".".into(),
        idempotency_key: None,
    }
}
fn session(executor: Arc<Echo>) -> (Scheduler, Vec<Arc<dyn Tool>>) {
    let baseline = SecurityBaseline {
        tools: context().policy,
        ..Default::default()
    };
    let owner = Scheduler::new(
        context().operation,
        SchedulerConfig {
            security: baseline.clone(),
            agents: [("worker".into(), baseline)].into_iter().collect(),
            ..Default::default()
        },
        executor,
        None,
    )
    .unwrap();
    let tools = build_subagent_task_tools(Arc::new(SubagentSession::new(owner.handle())), "worker");
    (owner, tools)
}
async fn invoke(tools: &[Arc<dyn Tool>], name: &str, id: &str, arguments: Value) -> (Value, bool) {
    let tool = tools
        .iter()
        .find(|tool| tool.definition().name == name)
        .unwrap();
    let output = tool
        .execute(
            &context(),
            ToolCall {
                id: id.into(),
                name: name.into(),
                arguments,
            },
        )
        .await
        .unwrap();
    let Content::Text { text } = &output.content[0] else {
        panic!("expected JSON text")
    };
    (serde_json::from_str(text).unwrap(), output.is_error)
}

#[tokio::test]
async fn dag_keys_are_local_to_each_call_and_graph_retains_resolved_edges() {
    let executor = Arc::new(Echo::default());
    let (owner, tools) = session(executor.clone());
    let input = json!({"mode":"background", "tasks":[
        {"key":"a", "message":"first"},
        {"key":"b", "message":"second", "depends_on":["a"]}
    ]});
    let (first, failed) = invoke(&tools, "subagent", "batch-1", input.clone()).await;
    assert!(!failed, "{first}");
    let (second, failed) = invoke(&tools, "subagent", "batch-2", input).await;
    assert!(!failed, "{second}");
    assert_ne!(
        first["task_ids_by_key"]["a"],
        second["task_ids_by_key"]["a"]
    );
    let (_, failed) = invoke(&tools, "subagent_wait", "wait", json!({})).await;
    assert!(!failed);
    let (graph, failed) = invoke(
        &tools,
        "subagent_status",
        "graph",
        json!({"detail":"graph"}),
    )
    .await;
    assert!(!failed);
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 4);
    assert_eq!(graph["edges"].as_array().unwrap().len(), 2);
    assert!(graph["edges"].as_array().unwrap().contains(
        &json!({"from":first["task_ids_by_key"]["a"], "to":first["task_ids_by_key"]["b"]})
    ));
    let (results, _) = invoke(
        &tools,
        "subagent_status",
        "results",
        json!({"detail":"results"}),
    )
    .await;
    assert!(
        results["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|task| !task["result"].as_str().unwrap().is_empty())
    );
    assert_eq!(executor.0.lock().unwrap().len(), 4);
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_batch_is_atomic_and_does_not_dispatch_valid_prefix() {
    let executor = Arc::new(Echo::default());
    let (owner, tools) = session(executor.clone());
    for (id, args) in [
        (
            "cycle",
            json!({"tasks":[{"key":"a","message":"first","depends_on":["b"]},{"key":"b","message":"second","depends_on":["a"]}]}),
        ),
        (
            "missing",
            json!({"tasks":[{"key":"a","message":"first"},{"key":"b","message":"second","depends_on":["missing"]}]}),
        ),
        (
            "duplicate",
            json!({"tasks":[{"key":"a","message":"first"},{"key":"a","message":"second"}]}),
        ),
        (
            "both",
            json!({"message":"single", "tasks":[{"key":"a","message":"first"}]}),
        ),
    ] {
        let (value, failed) = invoke(&tools, "subagent", id, args).await;
        assert!(failed, "{id}: {value}");
        assert!(owner.handle().list().is_empty());
    }
    assert!(executor.0.lock().unwrap().is_empty());
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn batch_access_cannot_override_call_read_only_and_dependency_forwarding_can_be_disabled() {
    let executor = Arc::new(Echo::default());
    let (owner, tools) = session(executor.clone());
    let (value, failed) = invoke(&tools, "subagent", "narrow", json!({
        "tool_access":"read-only", "tasks":[
            {"key":"a","message":"private-result"},
            {"key":"b","message":"isolated-task","tool_access":"full","depends_on":["a"],"include_dependency_results":false}
        ]
    })).await;
    assert!(!failed, "{value}");
    {
        let invocations = executor.0.lock().unwrap();
        assert_eq!(invocations.len(), 2);
        for invocation in invocations.iter() {
            assert_eq!(invocation.security.tools.access, AccessMode::ReadOnly);
            assert!(invocation.security.tools.allowed_mutating_tools.is_empty());
        }
        let second = invocations
            .iter()
            .find(|invocation| {
                serde_json::to_string(&invocation.request.input)
                    .unwrap()
                    .contains("isolated-task")
            })
            .unwrap();
        assert!(
            !serde_json::to_string(&second.request.input)
                .unwrap()
                .contains("private-result")
        );
    }
    owner.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn explicit_wait_any_returns_when_every_result_was_already_delivered() {
    let (owner, tools) = session(Arc::new(Echo::default()));
    let (_, failed) = invoke(&tools, "subagent", "sync", json!({"message":"done"})).await;
    assert!(!failed);
    let ids: Vec<_> = owner
        .handle()
        .list()
        .into_iter()
        .map(|task| task.id)
        .collect();
    let (value, failed) = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        invoke(
            &tools,
            "subagent_wait",
            "again",
            json!({"task_ids":ids,"wait_for":"any"}),
        ),
    )
    .await
    .expect("all-terminal explicit wait-any must not hang");
    assert!(!failed, "{value}");
    assert_eq!(value["finished"].as_array().unwrap().len(), 0);
    assert_eq!(value["previously_delivered"].as_array().unwrap().len(), 1);
    owner.shutdown().await.unwrap();
}
