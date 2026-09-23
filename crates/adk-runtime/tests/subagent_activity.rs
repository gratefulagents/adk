use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use adk_core::{
    BoxFuture, Content, Context, Error, Host, Model, ModelRequest, ModelResponse, Role, RunItem,
    Tool, ToolCall, ToolContext, ToolDefinition, ToolOutput,
};
use adk_runtime::{
    AgentConfig, CancellationToken, ModelBinding, Runner, RunnerChildExecutor, RunnerConfig,
    subagent::*,
};
use tokio::sync::{mpsc, oneshot};

struct ToolModel(AtomicUsize);
impl Model for ToolModel {
    fn provider(&self) -> &str {
        "fixture"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            let calls = match self.0.fetch_add(1, Ordering::SeqCst) {
                0 => vec![ToolCall {
                    id: "write".into(),
                    name: "Write".into(),
                    arguments: serde_json::json!({"file_path": "output.txt", "content": "secret contents"}),
                }],
                1 => vec![
                    ToolCall {
                        id: "read-output".into(),
                        name: "read_file".into(),
                        arguments: serde_json::json!({"path": "output.txt", "unused": "secret argument"}),
                    },
                    ToolCall {
                        id: "read-seed".into(),
                        name: "read_file".into(),
                        arguments: serde_json::json!({"path": "seed.txt"}),
                    },
                ],
                _ => vec![],
            };
            let items = if calls.is_empty() {
                vec![RunItem::Message {
                    message: adk_core::Message {
                        role: Role::Assistant,
                        content: vec![Content::Text {
                            text: "done".into(),
                        }],
                    },
                }]
            } else {
                calls
                    .into_iter()
                    .map(|call| RunItem::ToolCall { call })
                    .collect()
            };
            Ok(ModelResponse {
                raw: None,
                items,
                usage: Default::default(),
                end_turn: None,
                response_id: None,
                metadata: Default::default(),
            })
        })
    }
}

struct FileTool {
    fail_seed: bool,
    definition: ToolDefinition,
    started: mpsc::UnboundedSender<(String, oneshot::Sender<()>)>,
}
impl Tool for FileTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            if call.name == "Write" {
                std::fs::write(
                    context
                        .work_dir
                        .join(call.arguments["file_path"].as_str().unwrap()),
                    call.arguments["content"].as_str().unwrap(),
                )
                .unwrap();
            } else {
                let content = std::fs::read_to_string(
                    context
                        .work_dir
                        .join(call.arguments["path"].as_str().unwrap()),
                )
                .unwrap();
                assert!(!content.is_empty());
            }
            let (release, wait) = oneshot::channel();
            self.started.send((call.id.clone(), release)).unwrap();
            wait.await.unwrap();
            tokio::time::sleep(Duration::from_millis(5)).await;
            if self.fail_seed && call.id == "read-seed" {
                return Err(Error::new(
                    adk_core::ErrorCategory::Internal,
                    "fixture failure",
                ));
            }
            Ok(ToolOutput {
                content: vec![],
                is_error: false,
                should_pause: false,
            })
        })
    }
}
struct TestHost;
impl Host for TestHost {
    fn emit<'a>(
        &'a self,
        _: &'a Context,
        _: adk_core::RunEvent,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: adk_core::ApprovalRequest,
    ) -> BoxFuture<'a, Result<adk_core::ApprovalDecision, Error>> {
        Box::pin(async { Ok(adk_core::ApprovalDecision::Deny) })
    }
}
#[derive(Default)]
struct Store(Mutex<Vec<SchedulerCheckpoint>>);
impl SchedulerStore for Store {
    fn persist<'a>(
        &'a self,
        _: &'a Context,
        checkpoint: &'a SchedulerCheckpoint,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(checkpoint.clone());
            Ok(())
        })
    }
}
fn activity(handle: &SchedulerHandle) -> Activity {
    handle
        .status("activity", Detail::Activity)
        .unwrap()
        .activity
        .unwrap()
}
async fn completed_tools(handle: &SchedulerHandle, count: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while activity(handle).recent_activity.len() != count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn exercise(durable: bool, fail_seed: bool) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("seed.txt"), "seed").unwrap();
    let (started, mut starts) = mpsc::unbounded_channel();
    let mut agent = AgentConfig::new(
        "worker",
        ModelBinding::complete("fixture", Arc::new(ToolModel(AtomicUsize::new(0)))),
    );
    for name in ["Write", "read_file"] {
        agent.tools.push(Arc::new(FileTool {
            fail_seed,
            definition: ToolDefinition {
                name: name.into(),
                description: name.into(),
                input_schema: schemars::schema_for!(serde_json::Value),
                read_only: name == "read_file",
                requires_approval: false,
            },
            started: started.clone(),
        }));
    }
    let runner = Runner::new(
        agent,
        RunnerConfig {
            work_dir: dir.path().into(),
            ..Default::default()
        },
    )
    .unwrap();
    let store = Arc::new(Store::default());
    let mut security = SecurityBaseline::default();
    security.tools.access = adk_core::AccessMode::FullAccess;
    let owner = Scheduler::new(
        Context {
            run_id: "parent".into(),
            cancellation: Arc::new(CancellationToken::new()),
            deadline: None,
        },
        SchedulerConfig {
            agents: BTreeMap::from([("worker".into(), security.clone())]),
            security,
            ..Default::default()
        },
        Arc::new(RunnerChildExecutor::new(
            [("worker".into(), runner)].into(),
            Arc::new(TestHost),
        )),
        if durable { Some(store.clone()) } else { None },
    )
    .unwrap();
    let handle = owner.handle();
    let mut submission = Submission::new("worker", "exercise file tools");
    submission.id = "activity".into();
    submission.policy.tools.access = adk_core::AccessMode::FullAccess;
    handle.submit(submission).await.unwrap();
    let before = chrono::Utc::now();
    let (id, release) = tokio::time::timeout(Duration::from_secs(2), starts.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(id, "write");
    let current = activity(&handle);
    assert_eq!(current.current_tool, "Write");
    assert_eq!(current.current_tool_input, "output.txt");
    assert_eq!(current.current_step, "implementing");
    assert!(current.recent_activity.is_empty());
    release.send(()).unwrap();
    let (id, first) = tokio::time::timeout(Duration::from_secs(2), starts.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(id, "read-output");
    assert!(activity(&handle).files_written.contains("output.txt"));
    assert_eq!(activity(&handle).recent_activity.len(), 1);
    if durable {
        assert_eq!(activity(&handle).current_tool_input, "output.txt");
        first.send(()).unwrap();
        let (id, second) = tokio::time::timeout(Duration::from_secs(2), starts.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(id, "read-seed");
        assert!(activity(&handle).files_read.contains("output.txt"));
        assert_eq!(activity(&handle).recent_activity.len(), 2);
        assert_eq!(activity(&handle).current_tool_input, "seed.txt");
        second.send(()).unwrap();
    } else {
        let (id, second) = tokio::time::timeout(Duration::from_secs(2), starts.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(id, "read-seed");
        assert_eq!(activity(&handle).current_tool_input, "seed.txt");
        second.send(()).unwrap();
        completed_tools(&handle, 2).await;
        assert_eq!(activity(&handle).current_tool, "read_file");
        assert_eq!(activity(&handle).current_tool_input, "output.txt");
        first.send(()).unwrap();
    }
    let tasks = handle
        .wait(
            &["activity".into()],
            WaitMode::All,
            Some(Duration::from_secs(2)),
        )
        .await
        .unwrap();
    assert_eq!(
        tasks[0].status,
        if durable && fail_seed {
            TaskStatus::Failed
        } else {
            TaskStatus::Completed
        },
        "{tasks:?}"
    );
    let final_activity = activity(&handle);
    assert!(final_activity.current_tool.is_empty());
    assert!(final_activity.current_tool_input.is_empty());
    assert_eq!(final_activity.files_written, ["output.txt".into()].into());
    assert_eq!(
        final_activity.files_read,
        if fail_seed {
            ["output.txt".into()].into()
        } else {
            ["output.txt".into(), "seed.txt".into()].into()
        }
    );
    assert_eq!(final_activity.recent_activity.len(), 3);
    for entry in &final_activity.recent_activity {
        assert_eq!(entry.is_error, fail_seed && entry.summary == "seed.txt");
        assert!(entry.duration_ms >= 5);
        assert!(entry.timestamp >= before && entry.timestamp <= chrono::Utc::now());
        assert!(entry.summary == "output.txt" || entry.summary == "seed.txt");
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("output.txt")).unwrap(),
        "secret contents"
    );
    if durable {
        let snapshots = store.0.lock().unwrap();
        let activities: Vec<_> = snapshots
            .iter()
            .filter_map(|snapshot| snapshot.records.first()?.task.activity.as_ref())
            .collect();
        assert!(
            activities
                .iter()
                .any(|a| a.current_tool == "Write" && a.current_tool_input == "output.txt")
        );
        assert!(activities.iter().any(|a| a.current_tool == "read_file"
            && a.current_tool_input == "seed.txt"
            && a.files_read.contains("output.txt")
            && a.recent_activity.len() == 2));
        assert_eq!(*activities.last().unwrap(), &final_activity);
        assert!(
            !serde_json::to_string(&activities)
                .unwrap()
                .contains("secret")
        );
    }
    owner.shutdown().await.unwrap();
}
#[tokio::test]
async fn native_runner_persists_observed_file_activity() {
    exercise(true, false).await;
}
#[tokio::test]
async fn native_runner_preserves_overlapping_tool_activity() {
    exercise(false, false).await;
}

#[tokio::test]
async fn native_runner_persists_failed_tool_activity() {
    exercise(true, true).await;
}
#[tokio::test]
async fn native_runner_preserves_activity_after_parallel_tool_error() {
    exercise(false, true).await;
}
