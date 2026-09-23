use adk_core::*;
use adk_runtime::{subagent::*, *};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

fn message(role: Role, text: &str) -> RunItem {
    RunItem::Message {
        message: Message {
            role,
            content: vec![Content::Text { text: text.into() }],
        },
    }
}
fn answer(text: &str) -> ModelResponse {
    response(vec![message(Role::Assistant, text)])
}
fn response(items: Vec<RunItem>) -> ModelResponse {
    ModelResponse {
        raw: None,
        items,
        usage: Usage::default(),
        end_turn: None,
        response_id: None,
        metadata: Default::default(),
    }
}
fn call(name: &str, arguments: Value) -> ModelResponse {
    response(vec![RunItem::ToolCall {
        call: ToolCall {
            id: format!("call-{name}"),
            name: name.into(),
            arguments,
        },
    }])
}
fn context() -> Context {
    Context {
        run_id: "parent".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn request() -> RunRequest {
    RunRequest {
        input_provenance: Vec::new(),
        input: vec![message(Role::User, "delegate")],
        policy: RunPolicy::default(),
    }
}
#[derive(Default)]
struct TestHost;
impl Host for TestHost {
    fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { Ok(ApprovalDecision::Approve) })
    }
}
struct FakeModel {
    responses: Mutex<VecDeque<ModelResponse>>,
    requests: Mutex<Vec<ModelRequest>>,
    delay: Duration,
}
impl FakeModel {
    fn new(responses: Vec<ModelResponse>, delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(Vec::new()),
            delay,
        })
    }
}
impl Model for FakeModel {
    fn provider(&self) -> &str {
        "fake"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            tokio::time::sleep(self.delay).await;
            Ok(self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected model call"))
        })
    }
}
struct MutationTool(ToolDefinition);
impl Tool for MutationTool {
    fn definition(&self) -> &ToolDefinition {
        &self.0
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async { panic!("read-only child executed a mutation") })
    }
}
fn runner(name: &str, model: Arc<dyn Model>) -> Runner {
    let mut agent = AgentConfig::new(name, ModelBinding::complete("fake", model));
    agent.tools.push(Arc::new(MutationTool(ToolDefinition {
        name: "mutate".into(),
        description: "mutation".into(),
        input_schema: schemars::schema_for!(serde_json::Value),
        read_only: false,
        requires_approval: false,
    })));
    Runner::new(agent, RunnerConfig::default()).unwrap()
}
fn history_text(items: &[RunItem]) -> String {
    serde_json::to_string(items).unwrap()
}

async fn session(child: Arc<dyn Model>) -> (Scheduler, Arc<SubagentSession>) {
    session_with_runner(runner("worker", child), None).await
}
async fn session_with_runner(
    child: Runner,
    store: Option<Arc<dyn SchedulerStore>>,
) -> (Scheduler, Arc<SubagentSession>) {
    let executor = RunnerChildExecutor::new(
        [("worker".into(), child)].into_iter().collect(),
        Arc::new(TestHost),
    );
    let baseline = SecurityBaseline {
        tools: ToolPolicy {
            access: AccessMode::FullAccess,
            ..Default::default()
        },
        ..Default::default()
    };
    let owner = Scheduler::new(
        context(),
        SchedulerConfig {
            security: baseline.clone(),
            agents: [("worker".into(), baseline)].into_iter().collect(),
            ..Default::default()
        },
        Arc::new(executor),
        store,
    )
    .unwrap();
    let session = Arc::new(SubagentSession::new(owner.handle()));
    (owner, session)
}
fn parent(model: Arc<FakeModel>, session: Arc<SubagentSession>) -> Runner {
    let mut agent = AgentConfig::new("parent", ModelBinding::complete("fake", model));
    agent.tools = build_subagent_task_tools(session.clone(), "worker");
    Runner::new(
        agent,
        RunnerConfig {
            subagents: Some(session),
            ..Default::default()
        },
    )
    .unwrap()
}

#[tokio::test]
async fn background_final_answer_waits_and_delivers_exactly_once() {
    let child = FakeModel::new(vec![answer("child evidence")], Duration::from_millis(50));
    let (owner, session) = session(child.clone()).await;
    let model = FakeModel::new(
        vec![
            call(
                "subagent",
                json!({"message":"investigate", "mode":"background"}),
            ),
            answer("premature"),
            answer("synthesized"),
        ],
        Duration::ZERO,
    );
    let result = parent(model.clone(), session)
        .run(context(), request(), Arc::new(TestHost))
        .await
        .unwrap();
    assert_eq!(result.result.final_output, Some(json!("synthesized")));
    {
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(history_text(&requests[2].input).contains("child evidence"));
        assert_eq!(
            history_text(&result.result.history)
                .matches("child evidence")
                .count(),
            1
        );
        assert_eq!(child.requests.lock().unwrap().len(), 1);
    }
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn sync_result_is_not_injected_again_and_readonly_clamps_live_policy() {
    let child = FakeModel::new(
        vec![call("mutate", json!({})), answer("sync evidence")],
        Duration::ZERO,
    );
    let (owner, session) = session(child.clone()).await;
    let model = FakeModel::new(
        vec![
            call(
                "subagent",
                json!({"message":"investigate", "tool_access":"full"}),
            ),
            answer("done"),
        ],
        Duration::ZERO,
    );
    let mut request = request();
    request.policy.tools.access = AccessMode::ReadOnly;
    let result = parent(model.clone(), session)
        .run(context(), request, Arc::new(TestHost))
        .await
        .unwrap();
    assert_eq!(model.requests.lock().unwrap().len(), 2);
    assert!(
        child.requests.lock().unwrap()[0]
            .tools
            .iter()
            .all(|tool| tool.name != "mutate")
    );
    assert!(
        child.requests.lock().unwrap()[1]
            .input
            .iter()
            .any(|item| matches!(item, RunItem::ToolResult { output, .. } if output.is_error))
    );
    assert_eq!(
        history_text(&result.result.history)
            .matches("sync evidence")
            .count(),
        1
    );
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn sync_timeout_keeps_child_alive_across_runs_and_status_can_reread() {
    let child = FakeModel::new(vec![answer("late evidence")], Duration::from_millis(50));
    let (owner, session) = session(child).await;
    let model = FakeModel::new(
        vec![call(
            "subagent",
            json!({"message":"investigate", "timeout_ms":1}),
        )],
        Duration::ZERO,
    );
    let runner = parent(model, session.clone());
    let mut req = request();
    req.policy.tool_use = ToolUseBehavior::StopAfterTool;
    let result = runner
        .run(context(), req, Arc::new(TestHost))
        .await
        .unwrap();
    let text = history_text(&result.result.history);
    assert!(!text.contains("late evidence"));
    assert_eq!(result.result.status, RunStatus::Completed);
    tokio::time::sleep(Duration::from_millis(70)).await;
    let next = FakeModel::new(vec![answer("collected")], Duration::ZERO);
    let result = parent(next.clone(), session.clone())
        .run(context(), request(), Arc::new(TestHost))
        .await
        .unwrap();
    assert!(history_text(&next.requests.lock().unwrap()[0].input).contains("late evidence"));
    assert_eq!(result.result.final_output, Some(json!("collected")));
    let tools = build_subagent_task_tools(session, "worker");
    let context = ToolContext {
        operation: context(),
        work_dir: ".".into(),
        policy: ToolPolicy::default(),
        idempotency_key: None,
    };
    let status = tools[1]
        .execute(
            &context,
            ToolCall {
                id: "status".into(),
                name: "subagent_status".into(),
                arguments: json!({"detail":"results"}),
            },
        )
        .await
        .unwrap();
    assert!(
        serde_json::to_string(&status)
            .unwrap()
            .contains("late evidence")
    );
    owner.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn tool_policy_timeout_preserves_managed_pending_results() {
    for name in ["subagent", "subagent_wait", "specialist"] {
        let child = FakeModel::new(
            vec![answer("late policy evidence")],
            Duration::from_millis(50),
        );
        let (owner, session) = session(child).await;
        let arguments = if name == "subagent_wait" {
            owner
                .handle()
                .submit(Submission::new("worker", "investigate"))
                .await
                .unwrap();
            json!({})
        } else {
            json!({"message":"investigate"})
        };
        let model = FakeModel::new(vec![call(name, arguments)], Duration::ZERO);
        let mut agent = AgentConfig::new("parent", ModelBinding::complete("fake", model));
        agent.tools = build_subagent_task_tools(session.clone(), "worker");
        agent.tools.push(Arc::new(AgentAsTool::new(
            "specialist",
            "specialist",
            "worker",
            session.clone(),
        )));
        let runner = Runner::new(
            agent,
            RunnerConfig {
                subagents: Some(session.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let mut req = request();
        req.policy.tool_use = ToolUseBehavior::StopAfterTool;
        req.policy.tools.timeout = Some(Duration::from_millis(1));
        let result = runner
            .run(context(), req, Arc::new(TestHost))
            .await
            .unwrap();
        let output = result
            .result
            .history
            .iter()
            .find_map(|item| match item {
                RunItem::ToolResult { output, .. } => Some(output),
                _ => None,
            })
            .expect("pending tool result");
        assert!(!output.is_error, "{name}: {output:?}");
        let Content::Text { text } = &output.content[0] else {
            panic!("expected JSON text")
        };
        let text = text
            .strip_prefix("BEGIN UNTRUSTED TOOL OUTPUT\n")
            .unwrap()
            .strip_suffix("\nEND UNTRUSTED TOOL OUTPUT")
            .unwrap();
        let pending: Value = serde_json::from_str(text).unwrap();
        assert_eq!(pending["timed_out"], true, "{name}: {pending}");
        assert_eq!(pending["wait_complete"], false);
        let task_id = if name == "subagent_wait" {
            assert_eq!(pending["wait_for"], "all");
            pending["still_active"][0]["task_id"].as_str().unwrap()
        } else {
            assert_eq!(pending["agent"], "worker");
            pending["task_id"].as_str().unwrap()
        };
        assert!(
            !owner
                .handle()
                .status(task_id, Detail::Full)
                .unwrap()
                .status
                .is_terminal()
        );
        owner
            .handle()
            .wait(&[task_id.to_owned()], WaitMode::All, None)
            .await
            .unwrap();
        let next = FakeModel::new(vec![answer("collected")], Duration::ZERO);
        parent(next.clone(), session)
            .run(context(), request(), Arc::new(TestHost))
            .await
            .unwrap();
        assert!(
            history_text(&next.requests.lock().unwrap()[0].input).contains("late policy evidence")
        );
        owner.shutdown().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn nested_tool_policy_timeout_resumes_without_cancelling_children() {
    for max_concurrency in [1, 2] {
        for name in ["subagent", "subagent_wait", "specialist"] {
            let (foreign_owner, foreign_session) =
                session(FakeModel::new(vec![], Duration::ZERO)).await;
            let mut responses = Vec::new();
            if name == "subagent_wait" {
                responses.push(call(
                    "subagent",
                    json!({"message":"nested work", "mode":"background"}),
                ));
            }
            responses.extend([
                call(name, json!({"message":"nested work"})),
                answer("delegated"),
                answer("nested synthesis"),
            ]);
            let model = FakeModel::new(responses, Duration::ZERO);
            let mut agent =
                AgentConfig::new("worker", ModelBinding::complete("fake", model.clone()));
            agent.tools = build_subagent_task_tools(foreign_session.clone(), "leaf");
            agent.tools.push(Arc::new(AgentAsTool::new(
                "specialist",
                "specialist",
                "leaf",
                foreign_session.clone(),
            )));
            let child = Runner::new(
                agent,
                RunnerConfig {
                    subagents: Some(foreign_session),
                    ..Default::default()
                },
            )
            .unwrap();
            let leaf = runner(
                "leaf",
                FakeModel::new(
                    vec![answer("late nested evidence")],
                    Duration::from_millis(50),
                ),
            );
            let owner = Scheduler::new(
                context(),
                SchedulerConfig {
                    max_concurrency,
                    agents: [
                        ("worker".into(), SecurityBaseline::default()),
                        ("leaf".into(), SecurityBaseline::default()),
                    ]
                    .into_iter()
                    .collect(),
                    ..Default::default()
                },
                Arc::new(RunnerChildExecutor::new(
                    [("worker".into(), child), ("leaf".into(), leaf)]
                        .into_iter()
                        .collect(),
                    Arc::new(TestHost),
                )),
                None,
            )
            .unwrap();
            let handle = owner.handle();
            let root_model = FakeModel::new(
                vec![call(
                    "subagent",
                    json!({"message":"delegate", "mode":"background"}),
                )],
                Duration::ZERO,
            );
            let mut req = request();
            req.policy.tools.timeout = Some(Duration::from_millis(1));
            req.policy.tool_use = ToolUseBehavior::StopAfterTool;
            parent(root_model, Arc::new(SubagentSession::new(handle.clone())))
                .run(context(), req, Arc::new(TestHost))
                .await
                .unwrap();
            let worker_id = handle
                .list()
                .into_iter()
                .find(|task| task.agent_name == "worker")
                .unwrap()
                .id;
            tokio::time::timeout(
                Duration::from_secs(1),
                handle.wait(&[worker_id], WaitMode::All, None),
            )
            .await
            .unwrap()
            .unwrap();
            let tasks = handle.list();
            assert_eq!(tasks.len(), 2, "{name}, slots={max_concurrency}");
            assert!(
                tasks
                    .iter()
                    .all(|task| task.status == TaskStatus::Completed),
                "{name}, slots={max_concurrency}: {tasks:?}"
            );
            {
                let requests = model.requests.lock().unwrap();
                let after_wait = &requests[if name == "subagent_wait" { 2 } else { 1 }];
                let output = after_wait
                    .input
                    .iter()
                    .find_map(|item| match item {
                        RunItem::ToolResult {
                            call_id, output, ..
                        } if call_id == &format!("call-{name}") => Some(output),
                        _ => None,
                    })
                    .expect("nested wait result");
                assert!(!output.is_error, "{name}: {output:?}");
                let text = serde_json::to_string(output).unwrap();
                if max_concurrency == 2 {
                    assert!(text.contains("\\\"timed_out\\\":true"), "{name}: {text}");
                    assert!(
                        text.contains("\\\"wait_complete\\\":false"),
                        "{name}: {text}"
                    );
                    assert!(!text.contains("late nested evidence"), "{name}: {text}");
                }
                assert!(
                    history_text(&requests.last().unwrap().input).contains("late nested evidence")
                );
            }
            assert!(foreign_owner.handle().list().is_empty());
            owner.shutdown().await.unwrap();
            foreign_owner.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn agent_as_tool_shares_child_engine_and_explicit_parent_context_is_paired() {
    let child = FakeModel::new(vec![answer("first"), answer("second")], Duration::ZERO);
    let (owner, session) = session(child.clone()).await;
    let model = FakeModel::new(
        vec![
            call("specialist", json!({"message":"fresh task"})),
            answer("done"),
        ],
        Duration::ZERO,
    );
    let mut agent = AgentConfig::new("parent", ModelBinding::complete("fake", model));
    agent.tools = vec![Arc::new(AgentAsTool::new(
        "specialist",
        "specialist task",
        "worker",
        session.clone(),
    ))];
    let runner = Runner::new(
        agent,
        RunnerConfig {
            subagents: Some(session.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    runner
        .run(context(), request(), Arc::new(TestHost))
        .await
        .unwrap();
    let model = FakeModel::new(
        vec![
            call(
                "subagent",
                json!({"message":"shared task", "share_parent_context":true}),
            ),
            answer("done"),
        ],
        Duration::ZERO,
    );
    let mut req = request();
    req.input.push(message(Role::User, "parent context marker"));
    req.input_provenance = vec![
        ItemProvenance::Unknown,
        ItemProvenance::Agent {
            name: "original-parent".into(),
        },
    ];
    parent(model, session)
        .run(context(), req, Arc::new(TestHost))
        .await
        .unwrap();
    {
        let requests = child.requests.lock().unwrap();
        assert!(!history_text(&requests[0].input).contains("delegate"));
        assert!(history_text(&requests[1].input).contains("parent context marker"));
        assert_eq!(
            requests[1].input_provenance,
            vec![
                ItemProvenance::Unknown,
                ItemProvenance::Agent {
                    name: "original-parent".into()
                },
                ItemProvenance::Unattributed
            ]
        );
        assert!(
            !requests[1]
                .input
                .iter()
                .any(|item| matches!(item, RunItem::ToolCall { .. }))
        );
    }
    owner.shutdown().await.unwrap();
}

#[derive(Default)]
struct Checkpoints(Mutex<Vec<RunnerCheckpoint>>);
impl CheckpointStore for Checkpoints {
    fn persist<'a>(
        &'a self,
        _: &'a Context,
        checkpoint: &'a RunnerCheckpoint,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(checkpoint.clone());
            Ok(())
        })
    }
}

#[tokio::test]
async fn durable_runner_binds_native_scheduler_without_dynamic_turn_context() {
    let child = FakeModel::new(vec![answer("durable evidence")], Duration::from_millis(50));
    let (owner, session) = session_with_runner(
        runner("worker", child),
        Some(Arc::new(SchedulerCheckpoints::default())),
    )
    .await;
    let model = FakeModel::new(
        vec![
            call(
                "subagent",
                json!({"message":"investigate", "mode":"background"}),
            ),
            answer("premature"),
            answer("durable final"),
        ],
        Duration::ZERO,
    );
    let store = Arc::new(Checkpoints::default());
    let result = parent(model, session)
        .run_durable(
            context(),
            request(),
            Arc::new(TestHost),
            DurableRun::new(store.clone()),
        )
        .await
        .unwrap();
    assert_eq!(result.result.final_output, Some(json!("durable final")));
    {
        let checkpoints = store.0.lock().unwrap();
        let children = checkpoints.last().unwrap().children.as_ref().unwrap();
        assert_eq!(children["records"][0]["result_delivered"], true);
        assert_eq!(children["records"][0]["task"]["status"], "completed");
    }
    owner.shutdown().await.unwrap();
}

#[derive(Default)]
struct SchedulerCheckpoints(Mutex<Vec<SchedulerCheckpoint>>);
impl SchedulerStore for SchedulerCheckpoints {
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

struct InterruptModel {
    started: tokio::sync::Notify,
    requests: Mutex<Vec<ModelRequest>>,
}
impl Model for InterruptModel {
    fn provider(&self) -> &str {
        "fake"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            let first = {
                let mut requests = self.requests.lock().unwrap();
                requests.push(request);
                requests.len() == 1
            };
            if first {
                self.started.notify_one();
                std::future::pending().await
            } else {
                Ok(answer("steered"))
            }
        })
    }
}

#[tokio::test]
async fn steering_interrupts_pending_model_and_counts_attempts_once() {
    for durable in [false, true] {
        let model = Arc::new(InterruptModel {
            started: tokio::sync::Notify::new(),
            requests: Mutex::new(vec![]),
        });
        let store =
            durable.then(|| Arc::new(SchedulerCheckpoints::default()) as Arc<dyn SchedulerStore>);
        let (owner, session) = session_with_runner(runner("worker", model.clone()), store).await;
        let id = session
            .scheduler
            .submit(Submission::new("worker", "start"))
            .await
            .unwrap();
        model.started.notified().await;
        session
            .scheduler
            .steer(&id, "message-1", "new direction")
            .await
            .unwrap();
        session
            .scheduler
            .steer(&id, "message-1", "new direction")
            .await
            .unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            session
                .scheduler
                .wait(std::slice::from_ref(&id), WaitMode::All, None),
        )
        .await
        .unwrap()
        .unwrap();
        let task = session.scheduler.status(&id, Detail::Full).unwrap();
        assert_eq!(task.status, TaskStatus::Completed, "{:?}", task.error);
        assert_eq!(task.usage.turns, 2);
        assert_eq!(model.requests.lock().unwrap().len(), 2);
        assert_eq!(
            history_text(&model.requests.lock().unwrap()[1].input)
                .matches("new direction")
                .count(),
            1
        );
        let record = session.scheduler.snapshot().records.remove(0);
        assert_eq!(record.acknowledged_messages.len(), 1);
        if durable {
            assert_eq!(
                record
                    .durable_checkpoint
                    .unwrap()
                    .runtime
                    .unwrap()
                    .applied_child_messages()
                    .len(),
                1
            );
        }
        owner.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn denied_child_calls_do_not_consume_dispatch_budget() {
    let model = FakeModel::new(
        vec![call("mutate", json!({})), answer("done")],
        Duration::ZERO,
    );
    let (owner, session) = session(model).await;
    let mut submission = Submission::new("worker", "start");
    submission.policy.tools.access = AccessMode::ReadOnly;
    let task = session.scheduler.run(submission).await.unwrap();
    assert_eq!(task.status, TaskStatus::Completed);
    assert_eq!(task.usage.turns, 2);
    assert_eq!(task.usage.tool_calls, 0);
    owner.shutdown().await.unwrap();
}

struct RejectDelivery;
impl CheckpointStore for RejectDelivery {
    fn persist<'a>(
        &'a self,
        _: &'a Context,
        checkpoint: &'a RunnerCheckpoint,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            if checkpoint.runtime.as_ref().is_some_and(|runtime| {
                history_text(&runtime.result().history).contains("transaction evidence")
            }) {
                return Err(Error::new(
                    ErrorCategory::Internal,
                    "injected parent write failure",
                ));
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn failed_parent_checkpoint_does_not_consume_child_result() {
    let child = FakeModel::new(vec![answer("transaction evidence")], Duration::ZERO);
    let (owner, session) = session_with_runner(
        runner("worker", child),
        Some(Arc::new(SchedulerCheckpoints::default())),
    )
    .await;
    let task = session
        .scheduler
        .run(Submission::new("worker", "start"))
        .await
        .unwrap();
    assert_eq!(task.status, TaskStatus::Completed, "{:?}", task.error);
    let model = FakeModel::new(vec![answer("unused")], Duration::ZERO);
    let result = parent(model.clone(), session.clone())
        .run_durable(
            context(),
            request(),
            Arc::new(TestHost),
            DurableRun::new(Arc::new(RejectDelivery)),
        )
        .await;
    assert!(result.is_err());
    assert!(model.requests.lock().unwrap().is_empty());
    assert!(!session.scheduler.snapshot().records[0].result_delivered);
    let next = FakeModel::new(vec![answer("retried")], Duration::ZERO);
    parent(next.clone(), session.clone())
        .run(context(), request(), Arc::new(TestHost))
        .await
        .unwrap();
    assert_eq!(
        history_text(&next.requests.lock().unwrap()[0].input)
            .matches("transaction evidence")
            .count(),
        1
    );
    assert!(session.scheduler.snapshot().records[0].result_delivered);
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn native_durable_parent_refuses_unbacked_scheduler() {
    let (owner, session) = session(FakeModel::new(vec![], Duration::ZERO)).await;
    let model = FakeModel::new(vec![], Duration::ZERO);
    let error = parent(model, session)
        .run_durable(
            context(),
            request(),
            Arc::new(TestHost),
            DurableRun::new(Arc::new(Checkpoints::default())),
        )
        .await
        .err()
        .unwrap();
    assert!(error.error.info.message.contains("durable scheduler store"));
    owner.shutdown().await.unwrap();
}

struct DelayedEvents {
    events: VecDeque<ModelEvent>,
    paused: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    wait: bool,
}
impl ModelStream for DelayedEvents {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async move {
            if self.wait && self.events.len() == 1 {
                self.paused.notify_one();
                self.release.notified().await;
                self.wait = false;
            }
            Ok(self.events.pop_front())
        })
    }
}
struct VisibleModel {
    requests: Mutex<Vec<ModelRequest>>,
    paused: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
impl Model for VisibleModel {
    fn provider(&self) -> &str {
        "fake"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async { panic!("child must use genuine streaming") })
    }
}
impl StreamingModel for VisibleModel {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            let first = {
                let mut requests = self.requests.lock().unwrap();
                requests.push(request);
                requests.len() == 1
            };
            let mut events = VecDeque::new();
            if first {
                events.push_back(ModelEvent::TextDelta {
                    delta: "visible".into(),
                });
            }
            events.push_back(ModelEvent::Complete {
                response: answer(if first { "visible" } else { "steered" }),
            });
            Ok(Box::new(DelayedEvents {
                events,
                paused: self.paused.clone(),
                release: self.release.clone(),
                wait: first,
            }) as Box<dyn ModelStream>)
        })
    }
}

#[tokio::test]
async fn steering_after_visible_output_waits_for_safe_boundary() {
    let model = Arc::new(VisibleModel {
        requests: Mutex::new(vec![]),
        paused: Arc::new(tokio::sync::Notify::new()),
        release: Arc::new(tokio::sync::Notify::new()),
    });
    let child = Runner::new(
        AgentConfig::new("worker", ModelBinding::streaming("fake", model.clone())),
        RunnerConfig::default(),
    )
    .unwrap();
    let (owner, session) = session_with_runner(child, None).await;
    let id = session
        .scheduler
        .submit(Submission::new("worker", "start"))
        .await
        .unwrap();
    model.paused.notified().await;
    session
        .scheduler
        .steer(&id, "m1", "after visible")
        .await
        .unwrap();
    tokio::task::yield_now().await;
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    assert!(
        session.scheduler.snapshot().records[0]
            .acknowledged_messages
            .is_empty()
    );
    model.release.notify_one();
    tokio::time::timeout(
        Duration::from_secs(1),
        session
            .scheduler
            .wait(std::slice::from_ref(&id), WaitMode::All, None),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        session.scheduler.status(&id, Detail::Full).unwrap().status,
        TaskStatus::Completed
    );
    {
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(history_text(&requests[1].input).contains("visible"));
        assert_eq!(
            history_text(&requests[1].input)
                .matches("after visible")
                .count(),
            1
        );
    }
    owner.shutdown().await.unwrap();
}

struct RetryOnce(std::sync::atomic::AtomicUsize);
impl Model for RetryOnce {
    fn provider(&self) -> &str {
        "fake"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                Err(Error::new(ErrorCategory::Provider, "retry me"))
            } else {
                Ok(answer("retried child"))
            }
        })
    }
}
#[tokio::test]
async fn ordinary_child_retains_retry_policy_and_attempt_accounting() {
    let model = Arc::new(RetryOnce(std::sync::atomic::AtomicUsize::new(0)));
    let mut config = RunnerConfig::default();
    config.retry.max_retries = 1;
    config.retry.initial_delay = Duration::ZERO;
    config.retry.retryable = |_| true;
    let child = Runner::new(
        AgentConfig::new("worker", ModelBinding::complete("fake", model.clone())),
        config,
    )
    .unwrap();
    let (owner, session) = session_with_runner(child, None).await;
    let task = session
        .scheduler
        .run(Submission::new("worker", "start"))
        .await
        .unwrap();
    assert_eq!(task.status, TaskStatus::Completed, "{:?}", task.error);
    assert_eq!(task.usage.turns, 2);
    assert_eq!(model.0.load(std::sync::atomic::Ordering::SeqCst), 2);
    owner.shutdown().await.unwrap();
}

struct CountTool {
    definition: ToolDefinition,
    count: Arc<std::sync::atomic::AtomicUsize>,
}
impl Tool for CountTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn is_control_flow(&self) -> bool {
        true
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(ToolOutput {
                content: vec![Content::Text {
                    text: "counted".into(),
                }],
                is_error: false,
                should_pause: false,
            })
        })
    }
}
struct CrashAfterTool {
    saved: Mutex<Option<SchedulerCheckpoint>>,
    reached: tokio::sync::Notify,
    crash: std::sync::atomic::AtomicBool,
}
impl SchedulerStore for CrashAfterTool {
    fn persist<'a>(
        &'a self,
        _: &'a Context,
        checkpoint: &'a SchedulerCheckpoint,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            if self.crash.load(std::sync::atomic::Ordering::SeqCst)
                && checkpoint.records.iter().any(|record| {
                    record
                        .durable_checkpoint
                        .as_ref()
                        .is_some_and(|child| child.execution_boundary() == "tool_completed")
                })
            {
                *self.saved.lock().unwrap() = Some(checkpoint.clone());
                self.reached.notify_one();
                std::future::pending().await
            } else {
                Ok(())
            }
        })
    }
}
#[tokio::test]
async fn native_child_resume_preserves_tool_cursor_and_narrows_policy() {
    let model = FakeModel::new(
        vec![call("count", json!({})), answer("recovered")],
        Duration::ZERO,
    );
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut agent = AgentConfig::new("worker", ModelBinding::complete("fake", model.clone()));
    agent.tools.push(Arc::new(CountTool {
        definition: ToolDefinition {
            name: "count".into(),
            description: "count".into(),
            input_schema: schemars::schema_for!(Value),
            read_only: false,
            requires_approval: false,
        },
        count: count.clone(),
    }));
    let child = Runner::new(agent, RunnerConfig::default()).unwrap();
    let store = Arc::new(CrashAfterTool {
        saved: Mutex::new(None),
        reached: tokio::sync::Notify::new(),
        crash: std::sync::atomic::AtomicBool::new(true),
    });
    let (owner, session) = session_with_runner(child.clone(), Some(store.clone())).await;
    let mut submission = Submission::new("worker", "start");
    submission.policy.tools.access = AccessMode::FullAccess;
    let id = session.scheduler.submit(submission).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), store.reached.notified())
        .await
        .unwrap();
    let saved = store.saved.lock().unwrap().clone().unwrap();
    drop(session);
    drop(owner);
    tokio::task::yield_now().await;
    store
        .crash
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let baseline = SecurityBaseline::default();
    let executor = RunnerChildExecutor::new(
        [("worker".into(), child)].into_iter().collect(),
        Arc::new(TestHost),
    );
    let owner = Scheduler::new(
        context(),
        SchedulerConfig {
            security: baseline.clone(),
            agents: [("worker".into(), baseline)].into_iter().collect(),
            ..Default::default()
        },
        Arc::new(executor),
        Some(store),
    )
    .unwrap();
    let handle = owner.handle();
    handle
        .restore(&context(), serde_json::to_value(saved).unwrap())
        .await
        .unwrap();
    handle.resume_checkpoint(&id).await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(1),
        handle.wait(std::slice::from_ref(&id), WaitMode::All, None),
    )
    .await
    .unwrap()
    .unwrap();
    let task = handle.status(&id, Detail::Full).unwrap();
    assert_eq!(task.status, TaskStatus::Completed, "{:?}", task.error);
    assert_eq!(task.result, "recovered");
    assert_eq!(task.usage.turns, 2);
    assert_eq!(task.usage.tool_calls, 1);
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(model.requests.lock().unwrap().len(), 2);
    assert!(model.requests.lock().unwrap()[1].tools.is_empty());
    owner.shutdown().await.unwrap();
}

struct FractionalCost;
impl CostEstimator for FractionalCost {
    fn cost(&self, _: &str, _: &Usage) -> f64 {
        0.0000004
    }
}
#[tokio::test]
async fn cost_is_reported_cumulatively_not_rounded_per_response() {
    let model = FakeModel::new(
        vec![call("mutate", json!({})), answer("done")],
        Duration::ZERO,
    );
    let child = Runner::new(
        AgentConfig::new("worker", ModelBinding::complete("fake", model)),
        RunnerConfig {
            cost_estimator: Some(Arc::new(FractionalCost)),
            ..Default::default()
        },
    )
    .unwrap();
    let (owner, session) = session_with_runner(child, None).await;
    let task = session
        .scheduler
        .run(Submission::new("worker", "start"))
        .await
        .unwrap();
    assert_eq!(task.status, TaskStatus::Completed);
    assert_eq!(task.usage.cost_micros, 1);
    assert_eq!(session.scheduler.snapshot().usage.cost_micros, 1);
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn managed_child_tools_rebind_to_actual_scheduler_and_release_single_slot() {
    for durable in [false, true] {
        let (foreign_owner, foreign_session) =
            session(FakeModel::new(vec![], Duration::ZERO)).await;
        let model = FakeModel::new(
            vec![
                call(
                    "subagent",
                    json!({"message":"nested work", "agent_name":"leaf", "mode":"background"}),
                ),
                answer("delegated"),
                answer("nested synthesis"),
            ],
            Duration::ZERO,
        );
        let mut agent = AgentConfig::new("worker", ModelBinding::complete("fake", model));
        agent.tools = build_subagent_task_tools(foreign_session.clone(), "leaf");
        let child = Runner::new(
            agent,
            RunnerConfig {
                subagents: Some(foreign_session.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let leaf = runner(
            "leaf",
            FakeModel::new(vec![answer("leaf result")], Duration::ZERO),
        );
        let executor = RunnerChildExecutor::new(
            [("worker".into(), child), ("leaf".into(), leaf)]
                .into_iter()
                .collect(),
            Arc::new(TestHost),
        );
        let store = Arc::new(SchedulerCheckpoints::default());
        let owner = Scheduler::new(
            context(),
            SchedulerConfig {
                max_concurrency: 1,
                agents: [
                    ("worker".into(), SecurityBaseline::default()),
                    ("leaf".into(), SecurityBaseline::default()),
                ]
                .into_iter()
                .collect(),
                ..Default::default()
            },
            Arc::new(executor),
            durable.then(|| store.clone() as Arc<dyn SchedulerStore>),
        )
        .unwrap();
        let handle = owner.handle();
        let task = tokio::time::timeout(
            Duration::from_secs(1),
            handle.run(Submission::new("worker", "start")),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(task.status, TaskStatus::Completed, "{:?}", task.error);
        let ids: Vec<_> = handle.list().into_iter().map(|task| task.id).collect();
        assert_eq!(ids.len(), 2);
        tokio::time::timeout(
            Duration::from_secs(1),
            handle.wait(&ids, WaitMode::All, None),
        )
        .await
        .unwrap()
        .unwrap();
        let snapshot = handle.snapshot();
        let leaf = snapshot
            .records
            .iter()
            .find(|record| record.task.agent_name == "leaf")
            .unwrap();
        assert_eq!(leaf.depth, 2);
        assert_eq!(leaf.parent_id.as_deref(), Some(task.id.as_str()));
        assert_eq!(leaf.task.status, TaskStatus::Completed);
        assert!(leaf.result_delivered);
        assert!(foreign_session.scheduler.list().is_empty());
        if durable {
            let snapshots = store.0.lock().unwrap();
            let mut observed_atomic_delivery = false;
            for snapshot in snapshots.iter() {
                for record in &snapshot.records {
                    if let Some(runtime) = record
                        .durable_checkpoint
                        .as_ref()
                        .and_then(|c| c.runtime.as_ref())
                    {
                        for id in runtime.child_deliveries() {
                            observed_atomic_delivery = true;
                            assert!(
                                snapshot
                                    .records
                                    .iter()
                                    .find(|r| r.task.id == id)
                                    .unwrap()
                                    .result_delivered,
                                "child history checkpoint and descendant delivery must be atomic"
                            );
                        }
                    }
                }
            }
            assert!(observed_atomic_delivery);
        }
        owner.shutdown().await.unwrap();
        foreign_owner.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn failed_final_join_checkpoint_keeps_result_available() {
    let child = FakeModel::new(
        vec![answer("transaction evidence")],
        Duration::from_millis(50),
    );
    let (owner, session) = session_with_runner(
        runner("worker", child),
        Some(Arc::new(SchedulerCheckpoints::default())),
    )
    .await;
    let model = FakeModel::new(
        vec![
            call("subagent", json!({"message":"start", "mode":"background"})),
            answer("premature"),
        ],
        Duration::ZERO,
    );
    let error = parent(model.clone(), session.clone())
        .run_durable(
            context(),
            request(),
            Arc::new(TestHost),
            DurableRun::new(Arc::new(RejectDelivery)),
        )
        .await
        .err()
        .unwrap();
    assert!(
        error
            .error
            .info
            .message
            .contains("injected parent write failure")
    );
    assert_eq!(model.requests.lock().unwrap().len(), 2);
    assert!(!session.scheduler.snapshot().records[0].result_delivered);
    let next = FakeModel::new(vec![answer("retried join")], Duration::ZERO);
    parent(next.clone(), session)
        .run(context(), request(), Arc::new(TestHost))
        .await
        .unwrap();
    assert_eq!(
        history_text(&next.requests.lock().unwrap()[0].input)
            .matches("transaction evidence")
            .count(),
        1
    );
    owner.shutdown().await.unwrap();
}

struct PausingTool {
    definition: ToolDefinition,
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl Tool for PausingTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            self.started.notify_one();
            self.release.notified().await;
            Ok(ToolOutput {
                content: vec![Content::Text {
                    text: "effect completed".into(),
                }],
                is_error: false,
                should_pause: false,
            })
        })
    }
}
#[tokio::test]
async fn steering_does_not_interrupt_started_tool() {
    let model = FakeModel::new(
        vec![call("pause", json!({})), answer("done")],
        Duration::ZERO,
    );
    let tool = Arc::new(PausingTool {
        definition: ToolDefinition {
            name: "pause".into(),
            description: "pause".into(),
            input_schema: schemars::schema_for!(Value),
            read_only: true,
            requires_approval: false,
        },
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let mut agent = AgentConfig::new("worker", ModelBinding::complete("fake", model.clone()));
    agent.tools.push(tool.clone());
    let (owner, session) =
        session_with_runner(Runner::new(agent, RunnerConfig::default()).unwrap(), None).await;
    let id = session
        .scheduler
        .submit(Submission::new("worker", "start"))
        .await
        .unwrap();
    tool.started.notified().await;
    session
        .scheduler
        .steer(&id, "tool-message", "safe boundary directive")
        .await
        .unwrap();
    tokio::task::yield_now().await;
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    assert!(
        session.scheduler.snapshot().records[0]
            .acknowledged_messages
            .is_empty()
    );
    tool.release.notify_one();
    tokio::time::timeout(
        Duration::from_secs(1),
        session
            .scheduler
            .wait(std::slice::from_ref(&id), WaitMode::All, None),
    )
    .await
    .unwrap()
    .unwrap();
    let task = session.scheduler.status(&id, Detail::Full).unwrap();
    assert_eq!(task.status, TaskStatus::Completed);
    assert_eq!(task.usage.tool_calls, 1);
    let input = history_text(&model.requests.lock().unwrap()[1].input);
    assert!(input.contains("effect completed"));
    assert_eq!(input.matches("safe boundary directive").count(), 1);
    owner.shutdown().await.unwrap();
}
struct ProgressThenFailure(std::sync::atomic::AtomicUsize);
impl Model for ProgressThenFailure {
    fn provider(&self) -> &str {
        "fake"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                let mut response = answer(&format!("{} useful tail", "x".repeat(5000)));
                response.end_turn = Some(false);
                Ok(response)
            } else {
                Err(Error::new(ErrorCategory::Provider, "provider failed"))
            }
        })
    }
}
#[tokio::test]
async fn failed_child_retains_bounded_assistant_progress_tail() {
    let (owner, session) = session(Arc::new(ProgressThenFailure(
        std::sync::atomic::AtomicUsize::new(0),
    )))
    .await;
    let task = session
        .scheduler
        .run(Submission::new("worker", "start"))
        .await
        .unwrap();
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(task.result.chars().count(), 4096);
    assert!(task.result.ends_with("useful tail"));
    owner.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn output_returning_stop_after_tool_cannot_bypass_child_final_join() {
    for durable in [false, true] {
        for background in [false, true] {
            let child = FakeModel::new(vec![answer("joined evidence")], Duration::from_secs(1));
            let (owner, session) = session_with_runner(
                runner("worker", child),
                durable
                    .then(|| Arc::new(SchedulerCheckpoints::default()) as Arc<dyn SchedulerStore>),
            )
            .await;
            let model = FakeModel::new(
                vec![
                    call(
                        "subagent",
                        json!({"message":"work", "mode": if background { "background" } else { "sync" }, "timeout_ms": 1}),
                    ),
                    answer("joined final"),
                ],
                Duration::ZERO,
            );
            let mut agent = AgentConfig::new("parent", ModelBinding::complete("fake", model));
            agent.tools = build_subagent_task_tools(session.clone(), "worker");
            let runner = Runner::new(
                agent,
                RunnerConfig {
                    subagents: Some(session.clone()),
                    return_tool_output: true,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut input = request();
            input.policy.tool_use = ToolUseBehavior::StopAfterTool;
            let result = if durable {
                runner
                    .run_durable(
                        context(),
                        input,
                        Arc::new(TestHost),
                        DurableRun::new(Arc::new(Checkpoints::default())),
                    )
                    .await
            } else {
                runner.run(context(), input, Arc::new(TestHost)).await
            }
            .unwrap();
            assert_eq!(
                result.result.final_output,
                Some(json!("joined final")),
                "durable={durable} background={background}"
            );
            assert!(
                session
                    .scheduler
                    .list()
                    .iter()
                    .all(|task| task.status.is_terminal())
            );
            owner.shutdown().await.unwrap();
        }
    }
}

struct EarlyReturningParent;
impl ChildExecutor for EarlyReturningParent {
    fn execute<'a>(
        &'a self,
        invocation: ChildInvocation,
        control: ChildControl,
    ) -> BoxFuture<'a, Result<ChildOutcome, Error>> {
        Box::pin(async move {
            if invocation.task_id == "a" {
                let mut child = Submission::new("worker", "background descendant");
                child.id = "b".into();
                control.delegation_handle().submit(child).await?;
                Ok(ChildOutcome::completed("no-output parent turn ended"))
            } else {
                tokio::time::sleep(Duration::from_secs(1)).await;
                Ok(ChildOutcome::completed("descendant done"))
            }
        })
    }
}
#[tokio::test(start_paused = true)]
async fn final_join_includes_active_descendants_of_an_already_delivered_child() {
    let owner = Scheduler::new(
        context(),
        SchedulerConfig {
            max_concurrency: 1,
            agents: [("worker".into(), SecurityBaseline::default())].into(),
            ..Default::default()
        },
        Arc::new(EarlyReturningParent),
        None,
    )
    .unwrap();
    let handle = owner.handle();
    let mut child = Submission::new("worker", "parent");
    child.id = "a".into();
    handle.run(child).await.unwrap();
    handle.collect_ids(&["a".into()]).await.unwrap();
    assert!(
        !handle
            .status("b", Detail::Summary)
            .unwrap()
            .status
            .is_terminal()
    );
    let session = Arc::new(SubagentSession::new(handle.clone()));
    let result = parent(
        FakeModel::new(vec![answer("root final")], Duration::ZERO),
        session,
    )
    .run(context(), request(), Arc::new(TestHost))
    .await
    .unwrap();
    assert_eq!(result.result.final_output, Some(json!("root final")));
    assert_eq!(
        handle.status("b", Detail::Summary).unwrap().status,
        TaskStatus::Completed
    );
    owner.shutdown().await.unwrap();
}
