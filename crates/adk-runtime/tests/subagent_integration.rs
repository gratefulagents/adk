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
        snapshot_raw: None,
        snapshot_projection: None,
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
            raw_arguments: None,
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
async fn child_prompt_cache_key_uses_task_identity_not_parent_key() {
    use sha2::{Digest, Sha256};
    for (namespace, expected_namespace) in [
        (None, "parent"),
        (Some(""), "parent"),
        (Some(" \t\u{0085}"), "parent"),
        (Some("shared-run"), "shared-run"),
        (Some(" \u{2003}shared-run\u{3000}"), "shared-run"),
    ] {
        for parent_key in [None, Some("parent-key-must-not-leak".to_owned())] {
            let model = FakeModel::new(vec![answer("done"), answer("done")], Duration::ZERO);
            let child = Runner::new(
                AgentConfig::new("child", ModelBinding::complete("fake", model.clone())),
                RunnerConfig {
                    prompt_cache_key: parent_key,
                    prompt_cache_namespace: namespace.map(str::to_owned),
                    ..Default::default()
                },
            )
            .unwrap();
            let (owner, _) = session_with_runner(child, None).await;
            let handle = owner.handle();
            let mut keys = Vec::new();
            for task_id in ["task-stable-123", "task-stable-124"] {
                let mut submission = Submission::new("worker", "go");
                submission.id = task_id.into();
                let id = handle.submit(submission).await.unwrap();
                assert_eq!(id, task_id);
                let tasks = tokio::time::timeout(
                    Duration::from_secs(2),
                    handle.wait(std::slice::from_ref(&id), WaitMode::All, None),
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(tasks[0].status, TaskStatus::Completed);
                keys.push(format!(
                    "{:x}",
                    Sha256::digest(format!("{expected_namespace}\0{id}"))
                ));
            }
            {
                let requests = model.requests.lock().unwrap();
                assert_eq!(requests.len(), 2);
                for (request, key) in requests.iter().zip(&keys) {
                    assert_eq!(request.settings["prompt_cache_key"], *key);
                }
                assert_ne!(keys[0], keys[1]);
            }
            owner.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn child_default_cache_namespace_matches_go_root_trace() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-prompt-cache.json"
    ))
    .unwrap();
    let expected = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| {
            case["trace_id"] == "run-1"
                && case["logical"] == "task-stable-123"
                && case["namespace"] == ""
                && case["streaming"] == false
        })
        .unwrap();
    let model = FakeModel::new(vec![answer("done")], Duration::ZERO);
    let executor = RunnerChildExecutor::new(
        [("worker".into(), runner("worker", model.clone()))]
            .into_iter()
            .collect(),
        Arc::new(TestHost),
    );
    let owner = Scheduler::new(
        Context {
            run_id: "run-1".into(),
            ..context()
        },
        SchedulerConfig {
            agents: [("worker".into(), SecurityBaseline::default())]
                .into_iter()
                .collect(),
            ..Default::default()
        },
        Arc::new(executor),
        None,
    )
    .unwrap();
    let handle = owner.handle();
    let mut submission = Submission::new("worker", "go");
    submission.id = "task-stable-123".into();
    let id = handle.submit(submission).await.unwrap();
    let tasks = handle.wait(&[id], WaitMode::All, None).await.unwrap();
    assert_eq!(tasks[0].status, TaskStatus::Completed);
    assert_eq!(
        model.requests.lock().unwrap()[0].settings["prompt_cache_key"],
        expected["keys"][0]
    );
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn final_summary_waits_for_child_join_before_disabling_tools() {
    struct GatedChild(Arc<tokio::sync::Notify>);
    impl Model for GatedChild {
        fn provider(&self) -> &str {
            "test"
        }
        fn complete<'a>(
            &'a self,
            _: &'a Context,
            _: ModelRequest,
        ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
            Box::pin(async move {
                self.0.notified().await;
                Ok(answer("child evidence"))
            })
        }
    }
    struct ReleasingParent(Arc<FakeModel>, Arc<tokio::sync::Notify>);
    impl Model for ReleasingParent {
        fn provider(&self) -> &str {
            "test"
        }
        fn complete<'a>(
            &'a self,
            context: &'a Context,
            request: ModelRequest,
        ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
            self.1.notify_one();
            self.0.complete(context, request)
        }
    }
    let gate = Arc::new(tokio::sync::Notify::new());
    let (owner, session) = session(Arc::new(GatedChild(gate.clone()))).await;
    owner
        .handle()
        .submit(Submission::new("worker", "inspect"))
        .await
        .unwrap();
    let model = FakeModel::new(
        vec![answer("premature"), answer("synthesized")],
        Duration::ZERO,
    );
    let mut agent = AgentConfig::new(
        "parent",
        ModelBinding::complete("fake", Arc::new(ReleasingParent(model.clone(), gate))),
    );
    agent.tools = build_subagent_task_tools(session.clone(), "worker");
    let runner = Runner::new(
        agent,
        RunnerConfig {
            subagents: Some(session),
            force_final_summary_turn: true,
            ..Default::default()
        },
    )
    .unwrap();
    let mut req = request();
    req.policy.max_turns = std::num::NonZeroU32::new(1).unwrap();
    let result = runner
        .run(context(), req, Arc::new(TestHost))
        .await
        .unwrap();
    assert_eq!(result.result.final_output, Some(json!("synthesized")));
    {
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(!requests[0].tools.is_empty());
        assert!(!requests[0].instructions.contains("<final_turn>"));
        assert!(requests[1].tools.is_empty());
        assert!(requests[1].instructions.contains("<final_turn>"));
        assert!(history_text(&requests[1].input).contains("child evidence"));
    }
    owner.shutdown().await.unwrap();
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
                raw_arguments: None,
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
    struct GatedChild(Arc<tokio::sync::Notify>);
    impl Model for GatedChild {
        fn provider(&self) -> &str {
            "test"
        }
        fn complete<'a>(
            &'a self,
            _: &'a Context,
            _: ModelRequest,
        ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
            Box::pin(async move {
                self.0.notified().await;
                Ok(answer("late policy evidence"))
            })
        }
    }
    for name in ["subagent", "subagent_wait", "specialist"] {
        // Core deadlines use wall time, which can advance while Tokio time is paused.
        let release = Arc::new(tokio::sync::Notify::new());
        let child = Arc::new(GatedChild(release.clone()));
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
        release.notify_one();
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
    struct Leaf(Option<Arc<tokio::sync::Notify>>);
    impl Model for Leaf {
        fn provider(&self) -> &str {
            "fake"
        }
        fn complete<'a>(
            &'a self,
            _: &'a Context,
            _: ModelRequest,
        ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
            Box::pin(async move {
                if let Some(release) = &self.0 {
                    release.notified().await;
                } else {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Ok(answer("late nested evidence"))
            })
        }
    }
    struct ReleasingWorker {
        model: Arc<FakeModel>,
        release: Option<Arc<tokio::sync::Notify>>,
        call_id: String,
    }
    impl Model for ReleasingWorker {
        fn provider(&self) -> &str {
            "fake"
        }
        fn complete<'a>(
            &'a self,
            context: &'a Context,
            request: ModelRequest,
        ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
            if let Some(release) = &self.release
                && request.input.iter().any(|item| matches!(item, RunItem::ToolResult { call_id, .. } if call_id == &self.call_id))
            {
                release.notify_one();
            }
            self.model.complete(context, request)
        }
    }
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
            // Paused Tokio time cannot order a leaf delay against core wall-clock deadlines.
            let release = (max_concurrency == 2).then(|| Arc::new(tokio::sync::Notify::new()));
            let mut agent = AgentConfig::new(
                "worker",
                ModelBinding::complete(
                    "fake",
                    Arc::new(ReleasingWorker {
                        model: model.clone(),
                        release: release.clone(),
                        call_id: format!("call-{name}"),
                    }),
                ),
            );
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
            let leaf = runner("leaf", Arc::new(Leaf(release)));
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
async fn agent_as_tool_completed_output_matches_pinned_sdk() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/run-instructions/observations.json"
    ))
    .unwrap();
    for case in fixture["agent_tool_cases"].as_array().unwrap() {
        let child = FakeModel::new(
            vec![answer(case["answer"].as_str().unwrap())],
            Duration::ZERO,
        );
        let mut child_agent =
            AgentConfig::new("worker", ModelBinding::complete("fake", child.clone()));
        if case["structured"].as_bool().unwrap() {
            child_agent.output_schema =
                Some(adk_core::JsonDocument::new((json!(true)).to_string()).unwrap());
        }
        let child_runner = Runner::new(child_agent, RunnerConfig::default()).unwrap();
        let mut executor = RunnerChildExecutor::new(
            [("worker".into(), child_runner)].into_iter().collect(),
            Arc::new(TestHost),
        );
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let extractor = case["extractor"].as_str().unwrap().to_owned();
        if extractor != "none" {
            let calls = calls.clone();
            executor = executor.with_output_extractor(
                "worker",
                Arc::new(move |run| {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    assert_eq!(run.status, RunStatus::Completed);
                    assert_eq!(run.last_agent.as_deref(), Some("worker"));
                    assert!(!run.history.is_empty());
                    match extractor.as_str() {
                        "empty" => String::new(),
                        "json" => {
                            format!("json:{}", serde_json::to_string(&run.final_output).unwrap())
                        }
                        _ => format!("extracted:{}", run.final_text()),
                    }
                }),
            );
        }
        let baseline = SecurityBaseline::default();
        let owner = Scheduler::new(
            context(),
            SchedulerConfig {
                security: baseline.clone(),
                agents: [("worker".into(), baseline)].into_iter().collect(),
                ..Default::default()
            },
            Arc::new(executor),
            None,
        )
        .unwrap();
        let session = Arc::new(SubagentSession::new(owner.handle()));
        let model = FakeModel::new(
            vec![call("specialist", json!({"message":"work"}))],
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
                subagents: Some(session),
                output: adk_runtime::output::OutputPolicy {
                    untrusted: false,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        let mut req = request();
        req.policy.tool_use = ToolUseBehavior::StopAfterTool;
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
            .unwrap();
        assert_eq!(
            output.content,
            vec![Content::Text {
                text: case["content"].as_str().unwrap().into(),
            }],
            "{case}"
        );
        assert_eq!(
            output.is_error,
            case["is_error"].as_bool().unwrap(),
            "{case}"
        );
        assert_eq!(
            child.requests.lock().unwrap().len() as u64,
            case["requests"].as_u64().unwrap()
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst) as u64,
            case["extractor_calls"].as_u64().unwrap()
        );
        assert_eq!(owner.handle().list().len(), 1);
        assert_eq!(owner.handle().list()[0].status, TaskStatus::Completed);
        assert_eq!(
            owner.handle().list()[0].result,
            case["content"].as_str().unwrap()
        );
        owner.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn child_output_extractors_skip_failures_and_contain_panics() {
    struct DeniedModel;
    impl Model for DeniedModel {
        fn provider(&self) -> &str {
            "denied"
        }
        fn complete<'a>(
            &'a self,
            _: &'a Context,
            _: ModelRequest,
        ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
            Box::pin(async { Err(Error::new(ErrorCategory::PermissionDenied, "denied child")) })
        }
    }
    let child = FakeModel::new(vec![answer("first"), answer("second")], Duration::ZERO);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    let executor = RunnerChildExecutor::new(
        [
            ("worker".into(), runner("worker", child)),
            ("denied".into(), runner("denied", Arc::new(DeniedModel))),
        ]
        .into_iter()
        .collect(),
        Arc::new(TestHost),
    )
    .with_output_extractor(
        "denied",
        Arc::new(|_| panic!("failed child reached extractor")),
    )
    .with_output_extractor(
        "worker",
        Arc::new(move |run| {
            if observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                panic!("extractor failure");
            }
            format!("extracted:{}", run.final_text())
        }),
    );
    let baseline = SecurityBaseline::default();
    let owner = Scheduler::new(
        context(),
        SchedulerConfig {
            max_concurrency: 1,
            security: baseline.clone(),
            agents: [
                ("worker".into(), baseline.clone()),
                ("denied".into(), baseline),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        Arc::new(executor),
        None,
    )
    .unwrap();
    let handle = owner.handle();
    for (agent, status, expected) in [
        ("denied", TaskStatus::Failed, "denied child"),
        ("worker", TaskStatus::Failed, "child executor panicked"),
        ("worker", TaskStatus::Completed, "extracted:second"),
    ] {
        let id = handle.submit(Submission::new(agent, "work")).await.unwrap();
        let tasks = tokio::time::timeout(
            Duration::from_secs(2),
            handle.wait(&[id], WaitMode::All, None),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(tasks[0].status, status);
        let text = if status == TaskStatus::Completed {
            &tasks[0].result
        } else {
            tasks[0].error.as_ref().unwrap()
        };
        assert!(text.contains(expected), "{text}");
    }
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    owner.shutdown().await.unwrap();
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

struct LateChildInput(Mutex<usize>);
impl ImmediateInputFinalizer for LateChildInput {
    fn durable_key(&self) -> Option<&str> {
        Some("late-child-input-v1")
    }
    fn finalize<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<ImmediateInputBatch, Error>> {
        Box::pin(async move {
            let mut calls = self.0.lock().unwrap();
            *calls += 1;
            Ok(if *calls == 1 {
                ImmediateInputBatch {
                    items: vec![message(Role::User, "late child input")],
                    provenance: vec![ItemProvenance::Unattributed],
                }
            } else {
                ImmediateInputBatch::default()
            })
        })
    }
}

struct LateInputChildModel {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
    requests: Mutex<Vec<ModelRequest>>,
}
impl Model for LateInputChildModel {
    fn provider(&self) -> &str {
        "fake"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            let turn = {
                let mut requests = self.requests.lock().unwrap();
                requests.push(request);
                requests.len()
            };
            if turn == 2 {
                self.started.notify_one();
                self.release.notified().await;
            }
            Ok(answer(if turn == 1 { "candidate" } else { "done" }))
        })
    }
}

#[tokio::test]
async fn late_immediate_input_reopens_child_steering() {
    for durable in [false, true] {
        let model = Arc::new(LateInputChildModel {
            started: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            requests: Mutex::new(vec![]),
        });
        let finalizer = Arc::new(LateChildInput(Mutex::new(0)));
        let child = Runner::new(
            AgentConfig::new("worker", ModelBinding::complete("fake", model.clone())),
            RunnerConfig {
                immediate_input_finalizer: Some(finalizer.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let store =
            durable.then(|| Arc::new(SchedulerCheckpoints::default()) as Arc<dyn SchedulerStore>);
        let (owner, session) = session_with_runner(child, store).await;
        let id = session
            .scheduler
            .submit(Submission::new("worker", "start"))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), model.started.notified())
            .await
            .unwrap();
        assert_eq!(
            session
                .scheduler
                .status(&id, Detail::Summary)
                .unwrap()
                .status,
            TaskStatus::Running
        );
        session
            .scheduler
            .steer(&id, "after-late-input", "parent steering after late input")
            .await
            .unwrap();
        model.release.notify_one();
        tokio::time::timeout(
            Duration::from_secs(2),
            session
                .scheduler
                .wait(std::slice::from_ref(&id), WaitMode::All, None),
        )
        .await
        .unwrap()
        .unwrap();
        let record = session.scheduler.snapshot().records.remove(0);
        assert_eq!(
            record.task.status,
            TaskStatus::Completed,
            "{:?}",
            record.task.error
        );
        assert!(!record.accepting_messages);
        assert_eq!(record.acknowledged_messages.len(), 1);
        assert_eq!(*finalizer.0.lock().unwrap(), 2);
        {
            let requests = model.requests.lock().unwrap();
            assert_eq!(requests.len(), 3);
            assert!(history_text(&requests[1].input).contains("late child input"));
            assert_eq!(
                history_text(&requests[2].input)
                    .matches("parent steering after late input")
                    .count(),
                1
            );
        }
        assert!(
            session
                .scheduler
                .steer(&id, "too-late", "closed")
                .await
                .is_err()
        );
        owner.shutdown().await.unwrap();
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
#[tokio::test]
async fn child_engine_reserves_final_turn_without_changing_registered_runner() {
    for (turns, hallucinate) in [(1, false), (1, true), (2, false)] {
        let mut responses = Vec::new();
        if turns == 2 || hallucinate {
            responses.push(call("count", json!({})));
        }
        if !hallucinate {
            responses.push(answer("done"));
        }
        let model = FakeModel::new(responses, Duration::ZERO);
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
        let (owner, session) = session_with_runner(child.clone(), None).await;
        let mut submission = Submission::new("worker", "start");
        submission.policy.max_turns = std::num::NonZeroU32::new(turns).unwrap();
        submission.policy.tools.access = AccessMode::FullAccess;
        let task = session.scheduler.run(submission).await.unwrap();
        assert_eq!(
            task.status,
            if hallucinate {
                TaskStatus::Failed
            } else {
                TaskStatus::Completed
            }
        );
        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            usize::from(turns == 2)
        );
        {
            let requests = model.requests.lock().unwrap();
            assert_eq!(requests.len(), turns as usize);
            let last = requests.last().unwrap();
            assert!(last.tools.is_empty());
            assert!(last.instructions.contains("<final_turn>"));
            if turns == 2 {
                assert_eq!(requests[0].tools.len(), 1);
                assert!(!requests[0].instructions.contains("<final_turn>"));
            }
        }
        owner.shutdown().await.unwrap();
        model.responses.lock().unwrap().push_back(answer("direct"));
        let mut direct = request();
        direct.policy.max_turns = std::num::NonZeroU32::new(1).unwrap();
        direct.policy.tools.access = AccessMode::FullAccess;
        child
            .run(context(), direct, Arc::new(TestHost))
            .await
            .unwrap();
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.last().unwrap().tools.len(), 1);
        assert!(
            !requests
                .last()
                .unwrap()
                .instructions
                .contains("<final_turn>")
        );
    }
}

#[tokio::test]
async fn child_context_guidance_wraps_static_and_dynamic_instructions_without_mutation() {
    struct Dynamic;
    impl InstructionProvider for Dynamic {
        fn durable_key(&self) -> Option<&str> {
            Some("dynamic-child-test")
        }
        fn instructions<'a>(
            &'a self,
            context: InstructionContext<'a>,
        ) -> BoxFuture<'a, Result<String, Error>> {
            Box::pin(async move {
                assert!(context.agent.instructions.starts_with("static"));
                Ok("dynamic".into())
            })
        }
    }
    for dynamic in [false, true] {
        for durable in [false, true] {
            for work_dir in ["", "/tmp/child workspace"] {
                for access in [AccessMode::ReadOnly, AccessMode::FullAccess] {
                    let model = FakeModel::new(
                        vec![
                            answer("one"),
                            answer("two"),
                            answer("capped"),
                            answer("direct"),
                        ],
                        Duration::ZERO,
                    );
                    let mut agent =
                        AgentConfig::new("worker", ModelBinding::complete("fake", model.clone()));
                    agent.instructions = "static".into();
                    if dynamic {
                        agent.instruction_provider = Some(Arc::new(Dynamic));
                    }
                    agent.tool_access_ceiling = Some(access);
                    let child = Runner::new(
                        agent,
                        RunnerConfig {
                            work_dir: work_dir.into(),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    let store = durable.then(|| {
                        Arc::new(SchedulerCheckpoints::default()) as Arc<dyn SchedulerStore>
                    });
                    let (owner, session) = session_with_runner(child.clone(), store).await;
                    for (turns, effective_turns) in [(3, 3), (7, 7), (1000, 50)] {
                        let mut submission = Submission::new("worker", "inspect");
                        submission.policy.tools.access = AccessMode::FullAccess;
                        submission.policy.max_turns = std::num::NonZeroU32::new(turns).unwrap();
                        let task = session.scheduler.run(submission).await.unwrap();
                        assert_eq!(task.status, TaskStatus::Completed);
                        let requests = model.requests.lock().unwrap();
                        assert_eq!(
                            requests.last().unwrap().instructions,
                            format!(
                                "{}{}\n\n{}",
                                if dynamic { "dynamic" } else { "static" },
                                if work_dir.is_empty() {
                                    String::new()
                                } else {
                                    format!("\n\n{}", workspace_context(work_dir, access))
                                },
                                adk_codec::userinput::build_subagent_budget_context(i64::from(
                                    effective_turns
                                ))
                            )
                        );
                    }
                    owner.shutdown().await.unwrap();
                    child
                        .run(context(), request(), Arc::new(TestHost))
                        .await
                        .unwrap();
                    assert_eq!(
                        model.requests.lock().unwrap().last().unwrap().instructions,
                        if dynamic { "dynamic" } else { "static" }
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn child_context_does_not_leak_into_handoff_targets() {
    let model = FakeModel::new(
        vec![call("transfer_to_target", json!({})), answer("done")],
        Duration::ZERO,
    );
    let mut target = AgentConfig::new("target", ModelBinding::complete("fake", model.clone()));
    target.instructions = "target instructions".into();
    let mut initial = AgentConfig::new("worker", ModelBinding::complete("fake", model.clone()));
    initial.handoffs.push(Handoff::new(Arc::new(target)));
    let child = Runner::new(initial, RunnerConfig::default()).unwrap();
    let (owner, session) = session_with_runner(child, None).await;
    let task = session
        .scheduler
        .run(Submission::new("worker", "inspect"))
        .await
        .unwrap();
    assert_eq!(task.status, TaskStatus::Completed);
    {
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].instructions.contains("<sub_agent_budget>"));
        assert!(requests[0].instructions.contains("<environment>"));
        assert_eq!(requests[1].instructions, "target instructions");
    }
    owner.shutdown().await.unwrap();
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
    assert!(
        model.requests.lock().unwrap()[0]
            .instructions
            .contains("Tool access: full")
    );
    assert!(
        model.requests.lock().unwrap()[1]
            .instructions
            .contains("Tool access: read-only")
    );
    owner.shutdown().await.unwrap();
}

struct FractionalCost(Mutex<VecDeque<Option<f64>>>);
impl CostEstimator for FractionalCost {
    fn cost(&self, _: &str, _: &Usage) -> f64 {
        panic!("child prices must retain certainty")
    }
    fn estimate_cost(&self, _: &str, _: &Usage) -> Option<f64> {
        self.0
            .lock()
            .unwrap()
            .pop_front()
            .expect("duplicate estimate")
    }
}
#[tokio::test]
async fn cost_is_reported_cumulatively_not_rounded_per_response() {
    for estimates in [
        [Some(0.0000004), Some(0.0000004)],
        [Some(0.0), Some(0.0)],
        [Some(0.0000004), None],
        [None, Some(0.0000004)],
        [None, None],
    ] {
        let costs = Arc::new(FractionalCost(Mutex::new(estimates.into())));
        let model = FakeModel::new(
            vec![call("mutate", json!({})), answer("done")],
            Duration::ZERO,
        );
        let child = Runner::new(
            AgentConfig::new("worker", ModelBinding::complete("fake", model)),
            RunnerConfig {
                cost_estimator: Some(costs.clone()),
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
        let expected = estimates.into_iter().flatten().sum::<f64>();
        let micros = (expected * 1_000_000.0).ceil() as u64;
        assert_eq!(task.usage.cost_micros, micros);
        assert_eq!(task.cost_usd, expected);
        assert_eq!(task.cost_known, estimates.iter().all(Option::is_some));
        assert_eq!(session.scheduler.snapshot().usage.cost_micros, micros);
        assert!(costs.0.lock().unwrap().is_empty());
        owner.shutdown().await.unwrap();
    }
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
struct FindingsThenFailure(Mutex<Option<Vec<RunItem>>>);
impl Model for FindingsThenFailure {
    fn provider(&self) -> &str {
        "fake"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            if let Some(mut items) = self.0.lock().unwrap().take() {
                items.extend(call("echo", json!({"text":"echo completed"})).items);
                Ok(response(items))
            } else {
                assert!(request.input.iter().any(|item| matches!(
                    item,
                    RunItem::ToolResult { call_id, output }
                        if call_id == "call-echo" && !output.is_error
                            && output.content.iter().any(|content| matches!(
                                content, Content::Text { text } if text.contains("echo completed")
                            ))
                )));
                Err(Error::new(ErrorCategory::Provider, "provider failed"))
            }
        })
    }
}
struct EchoTool(ToolDefinition);
impl Tool for EchoTool {
    fn definition(&self) -> &ToolDefinition {
        &self.0
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            Ok(ToolOutput {
                content: vec![Content::Text {
                    text: call.arguments["text"].as_str().unwrap().into(),
                }],
                is_error: false,
                should_pause: false,
            })
        })
    }
}
fn findings_then_failure(items: Vec<RunItem>) -> Runner {
    let mut agent = AgentConfig::new(
        "worker",
        ModelBinding::complete(
            "fake",
            Arc::new(FindingsThenFailure(Mutex::new(Some(items)))),
        ),
    );
    agent.tools.push(Arc::new(EchoTool(ToolDefinition {
        name: "echo".into(),
        description: "echo".into(),
        input_schema: schemars::schema_for!(Value),
        read_only: true,
        requires_approval: false,
    })));
    Runner::new(agent, RunnerConfig::default()).unwrap()
}

#[tokio::test]
async fn failed_child_without_partial_findings_keeps_bare_error() {
    let (owner, session) =
        session(Arc::new(RetryOnce(std::sync::atomic::AtomicUsize::new(0)))).await;
    let task = session
        .scheduler
        .run(Submission::new("worker", "input is not progress"))
        .await
        .unwrap();
    assert_eq!(task.status, TaskStatus::Failed);
    assert!(task.result.is_empty());
    assert_eq!(
        task.error.as_deref(),
        Some("agent \"worker\" failed: model call failed on turn 0: retry me")
    );
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_child_error_selects_last_nonblank_non_system_message() {
    for (items, expected) in [
        (vec![], None),
        (
            vec![
                message(Role::Assistant, " \n\t"),
                message(Role::Assistant, "  [SYSTEM] budget reminder"),
            ],
            None,
        ),
        (
            vec![
                message(Role::Assistant, "first finding"),
                message(Role::Assistant, "  latest finding \n"),
                message(Role::Assistant, "\n [SYSTEM] budget reminder"),
                message(Role::Assistant, " \t"),
            ],
            Some("latest finding"),
        ),
        (
            vec![
                message(Role::Assistant, "older finding"),
                RunItem::PhasedMessage {
                    message: Message {
                        role: Role::Assistant,
                        content: vec![Content::Text {
                            text: "  phased finding [SYSTEM] is not a prefix \n".into(),
                        }],
                    },
                    phase: "commentary".into(),
                },
                message(Role::Assistant, "[SYSTEM] final reminder"),
            ],
            Some("phased finding [SYSTEM] is not a prefix"),
        ),
    ] {
        let (owner, session) = session_with_runner(findings_then_failure(items), None).await;
        let task = session
            .scheduler
            .run(Submission::new("worker", "input is not progress"))
            .await
            .unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.usage.tool_calls, 1);
        let mut error =
            "agent \"worker\" failed: model call failed on turn 1: provider failed".to_owned();
        if let Some(expected) = expected {
            error.push_str("\nPartial progress before the run ended:\n");
            error.push_str(expected);
        }
        assert_eq!(task.error.as_deref(), Some(error.as_str()));
        owner.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn failed_child_error_truncates_unicode_progress_with_exact_counted_elision() {
    for length in [1600, 1601, 5000] {
        let finding = format!("{}{}", "界".repeat(1100), "🦀".repeat(length - 1100));
        let expected = if length == 1600 {
            finding.clone()
        } else {
            let tail = if length == 1601 {
                format!("{}{}", "界".repeat(20), "🦀".repeat(501))
            } else {
                "🦀".repeat(521)
            };
            format!(
                "{}\n…[elided {} chars]…\n{tail}",
                "界".repeat(1042),
                length - 1563
            )
        };
        let (owner, session) = session_with_runner(
            findings_then_failure(vec![RunItem::PhasedMessage {
                message: Message {
                    role: Role::Assistant,
                    content: vec![Content::Text {
                        text: format!(" \n{finding}\t "),
                    }],
                },
                phase: "commentary".into(),
            }]),
            None,
        )
        .await;
        let task = session
            .scheduler
            .run(Submission::new("worker", "start"))
            .await
            .unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(
            task.error.unwrap(),
            format!(
                "agent \"worker\" failed: model call failed on turn 1: provider failed\nPartial progress before the run ended:\n{expected}"
            )
        );
        owner.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn failed_child_findings_reach_named_and_managed_sync_parent_requests_and_summary() {
    for tool_name in ["specialist", "subagent"] {
        let (owner, session) = session_with_runner(
            findings_then_failure(vec![message(Role::Assistant, "found the root cause")]),
            None,
        )
        .await;
        let model = FakeModel::new(
            vec![
                call(tool_name, json!({"message":"inspect", "mode":"sync"})),
                call("subagent_status", json!({"detail":"summary"})),
                answer("findings recovered"),
            ],
            Duration::ZERO,
        );
        let mut agent = AgentConfig::new("parent", ModelBinding::complete("fake", model.clone()));
        agent.tools = build_subagent_task_tools(session.clone(), "worker");
        agent.tools.push(Arc::new(AgentAsTool::new(
            "specialist",
            "specialist task",
            "worker",
            session.clone(),
        )));
        let runner = Runner::new(
            agent,
            RunnerConfig {
                subagents: Some(session),
                output: adk_runtime::output::OutputPolicy {
                    untrusted: false,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        runner
            .run(context(), request(), Arc::new(TestHost))
            .await
            .unwrap();
        {
            let requests = model.requests.lock().unwrap();
            assert_eq!(requests.len(), 3);
            let error = "agent \"worker\" failed: model call failed on turn 1: provider failed\nPartial progress before the run ended:\nfound the root cause";
            for (index, name) in [(1, tool_name), (2, "subagent_status")] {
                let output = requests[index]
                    .input
                    .iter()
                    .find_map(|item| match item {
                        RunItem::ToolResult { call_id, output }
                            if call_id == &format!("call-{name}") =>
                        {
                            Some(output)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(output.is_error, index == 1, "{tool_name}: {output:?}");
                let Content::Text { text } = &output.content[0] else {
                    panic!("expected text output")
                };
                let value: Value = serde_json::from_str(text).unwrap();
                let task = if index == 1 {
                    &value
                } else {
                    &value["tasks"][0]
                };
                assert_eq!(task["status"], "failed");
                assert_eq!(task["error"], error);
                if index == 2 {
                    assert!(task.get("result").is_none());
                }
            }
        }
        owner.shutdown().await.unwrap();
    }
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

#[tokio::test]
async fn successful_child_projects_frozen_prices_but_errors_and_budgets_keep_actual_charges() {
    struct Overloaded;
    impl Model for Overloaded {
        fn provider(&self) -> &str {
            "fake"
        }
        fn complete<'a>(
            &'a self,
            _: &'a Context,
            _: ModelRequest,
        ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
            Box::pin(async { Err(Error::new(ErrorCategory::Provider, "overloaded")) })
        }
        fn retry_advice(&self, _: &Error) -> Option<ModelRetryAdvice> {
            Some(ModelRetryAdvice {
                should_retry: true,
                retry_after: Duration::ZERO,
                reason: "overloaded".into(),
            })
        }
    }
    struct DisplayCost {
        prices: Mutex<VecDeque<f64>>,
        display: f64,
        known: bool,
        calls: Mutex<Vec<(String, Usage)>>,
    }
    impl CostEstimator for DisplayCost {
        fn cost(&self, model: &str, _: &Usage) -> f64 {
            assert_eq!(model, "active");
            self.prices
                .lock()
                .unwrap()
                .pop_front()
                .expect("response re-estimated")
        }
        fn estimate_display_cost(&self, model: &str, usage: &Usage) -> ResponseCostSummary {
            self.calls
                .lock()
                .unwrap()
                .push((model.into(), usage.clone()));
            ResponseCostSummary {
                cost_usd: self.display,
                cost_known: self.known,
            }
        }
    }
    struct CompactionPrice {
        amount: f64,
        known: bool,
    }
    impl Compactor for CompactionPrice {
        fn compact<'a>(
            &'a self,
            _: &'a Context,
            request: CompactionRequest,
        ) -> BoxFuture<'a, Result<CompactedHistory, Error>> {
            assert_eq!(request.model, "active");
            Box::pin(async move {
                Ok(CompactedHistory {
                    history: vec![message(Role::User, "summary")],
                    history_provenance: vec![],
                    context_tokens: 1,
                    usage: Usage {
                        requests: 1,
                        input_tokens: 20,
                        ..Default::default()
                    },
                    cost: self.amount,
                    cost_known: self.known,
                })
            })
        }
    }
    for (failed, compaction_known, display, display_known, response_price, compaction_price) in [
        (false, true, 1.0, true, 0.375, 0.25),
        (false, false, 1.0, true, 0.375, 0.25),
        (true, true, 1.0, true, 0.375, 0.25),
        (true, false, 1.0, true, 0.375, 0.25),
        (false, true, 0.25, false, 0.375, 0.25),
        (false, true, -1.0, true, 0.375, 0.25),
        (false, true, f64::NAN, true, 0.375, 0.25),
        (false, true, f64::INFINITY, true, 0.375, 0.25),
        (false, true, 1.0, true, 0.0000004, 1e12),
    ] {
        let costs = Arc::new(DisplayCost {
            prices: Mutex::new(
                if failed {
                    vec![response_price; 2]
                } else {
                    vec![response_price, response_price, 0.0]
                }
                .into(),
            ),
            display,
            known: display_known,
            calls: Mutex::new(vec![]),
        });
        let mut more = answer("more");
        more.end_turn = Some(false);
        more.usage = Usage {
            requests: 1,
            input_tokens: 10,
            context_tokens: Some(12),
            ..Default::default()
        };
        let mut done = answer("done");
        done.usage = Usage {
            requests: 1,
            input_tokens: 10,
            ..Default::default()
        };
        let active = FakeModel::new(vec![more.clone(), more, done], Duration::ZERO);
        let mut agent = AgentConfig::new(
            "worker",
            ModelBinding::complete("original", Arc::new(Overloaded)),
        );
        agent
            .fallbacks
            .push(ModelBinding::complete("active", active));
        let child = Runner::new(
            agent,
            RunnerConfig {
                cost_estimator: Some(costs.clone()),
                limits: Limits {
                    max_cost: failed.then_some(1.1),
                    ..Default::default()
                },
                compaction: Some(CompactionConfig {
                    trigger_tokens: 10,
                    target_tokens: 5,
                    compactor: Arc::new(CompactionPrice {
                        amount: compaction_price,
                        known: compaction_known,
                    }),
                }),
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
        assert_eq!(
            task.status,
            if failed {
                TaskStatus::Failed
            } else {
                TaskStatus::Completed
            },
            "{:?}",
            task.error
        );
        let actual = ((response_price + compaction_price) + response_price) + compaction_price;
        assert_eq!(task.usage.cost_micros, (actual * 1_000_000.0).ceil() as u64);
        assert_eq!(
            session.scheduler.snapshot().usage.cost_micros,
            task.usage.cost_micros
        );
        assert_eq!(task.usage.tokens, if failed { 60 } else { 70 });
        if failed {
            assert_eq!((task.cost_usd, task.cost_known), (actual, compaction_known));
            assert!(costs.calls.lock().unwrap().is_empty());
        } else {
            let valid = display.is_finite() && display >= 0.0;
            assert_eq!(
                task.cost_usd,
                response_price * 2.0 + if valid { display } else { 0.0 }
            );
            assert_eq!(task.cost_known, valid && display_known);
            assert_eq!(
                *costs.calls.lock().unwrap(),
                vec![(
                    "original".into(),
                    Usage {
                        requests: 2,
                        input_tokens: 40,
                        ..Default::default()
                    }
                )]
            );
        }
        assert!(costs.prices.lock().unwrap().is_empty());
        owner.shutdown().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn cancellation_observation_does_not_publish_results_or_finish_parent_join() {
    struct CancelBarrier {
        blocked: std::sync::atomic::AtomicBool,
        release: tokio::sync::Semaphore,
    }
    impl SchedulerStore for CancelBarrier {
        fn persist<'a>(
            &'a self,
            _: &'a Context,
            checkpoint: &'a SchedulerCheckpoint,
        ) -> BoxFuture<'a, Result<(), Error>> {
            Box::pin(async move {
                if checkpoint
                    .records
                    .iter()
                    .any(|r| r.task.status == TaskStatus::Cancelled)
                    && !self.blocked.swap(true, std::sync::atomic::Ordering::SeqCst)
                {
                    self.release.acquire().await.unwrap().forget();
                }
                Ok(())
            })
        }
    }
    let store = Arc::new(CancelBarrier {
        blocked: std::sync::atomic::AtomicBool::new(false),
        release: tokio::sync::Semaphore::new(0),
    });
    let child = FakeModel::new(vec![answer("must not finish")], Duration::from_secs(3600));
    let (owner, session) =
        session_with_runner(runner("worker", child.clone()), Some(store.clone())).await;
    let id = session
        .scheduler
        .submit(Submission::new("worker", "work"))
        .await
        .unwrap();
    while child.requests.lock().unwrap().is_empty() {
        tokio::task::yield_now().await;
    }
    let committed = serde_json::to_value(session.scheduler.snapshot()).unwrap();
    let cancel = session.scheduler.cancel(&id);
    tokio::pin!(cancel);
    assert!(futures_util::poll!(&mut cancel).is_pending());
    assert!(store.blocked.load(std::sync::atomic::Ordering::SeqCst));
    let tools = build_subagent_task_tools(session.clone(), "worker");
    let tool_context = ToolContext {
        operation: context(),
        work_dir: ".".into(),
        policy: ToolPolicy::default(),
        idempotency_key: None,
    };
    for (name, arguments) in [
        ("subagent_status", json!({"detail":"summary"})),
        ("subagent_status", json!({"detail":"graph"})),
        ("subagent_status", json!({"detail":"results"})),
        ("subagent_wait", json!({"task_ids":[id], "timeout_ms":1})),
    ] {
        let tool = tools.iter().find(|t| t.definition().name == name).unwrap();
        let result = tool
            .execute(
                &tool_context,
                ToolCall {
                    raw_arguments: None,
                    id: "observe".into(),
                    name: name.into(),
                    arguments: arguments.clone(),
                },
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        let Content::Text { text } = &result.content[0] else {
            panic!("expected text")
        };
        let value: Value = serde_json::from_str(text).unwrap();
        match arguments["detail"].as_str() {
            Some("summary") => assert_eq!(value["tasks"][0]["status"], "cancelled"),
            Some("graph") => assert_eq!(value["nodes"][0]["status"], "cancelled"),
            Some("results") => {
                assert_eq!(value["results"], json!([]));
                assert_eq!(value["still_active"], json!([id]));
            }
            _ => {
                assert_eq!(value["wait_complete"], false);
                assert_eq!(value["timed_out"], true);
                assert_eq!(value["still_active"][0]["status"], "running");
                assert!(value.get("finished").is_none());
            }
        }
    }
    let model = FakeModel::new(vec![answer("premature"), answer("joined")], Duration::ZERO);
    let runner = parent(model.clone(), session.clone());
    let run = runner.run(context(), request(), Arc::new(TestHost));
    tokio::pin!(run);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), &mut run)
            .await
            .is_err()
    );
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    assert!(!history_text(&model.requests.lock().unwrap()[0].input).contains("Sub-agent result"));
    assert_eq!(
        serde_json::to_value(session.scheduler.snapshot()).unwrap(),
        committed
    );
    assert!(futures_util::poll!(&mut cancel).is_pending());
    store.release.add_permits(1);
    cancel.await.unwrap();
    let result = run.await.unwrap();
    assert_eq!(result.result.final_output, Some(json!("joined")));
    {
        let requests = model.requests.lock().unwrap();
        assert!(history_text(&requests[1].input).contains("cancellation requested"));
    }
    assert!(session.scheduler.snapshot().records[0].result_delivered);
    owner.shutdown().await.unwrap();
}
