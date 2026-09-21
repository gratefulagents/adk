use adk_core::*;
use adk_runtime::*;
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct Store {
    checkpoints: Mutex<Vec<RunnerCheckpoint>>,
    fail: Mutex<Option<(String, bool)>>,
}
impl Store {
    fn latest(&self) -> RunnerCheckpoint {
        self.checkpoints.lock().unwrap().last().unwrap().clone()
    }
    fn at(&self, boundary: &str) -> RunnerCheckpoint {
        self.checkpoints
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.execution_boundary() == boundary)
            .unwrap()
            .clone()
    }
}
impl CheckpointStore for Store {
    fn persist<'a>(
        &'a self,
        _: &'a Context,
        checkpoint: &'a RunnerCheckpoint,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let fault = self
                .fail
                .lock()
                .unwrap()
                .clone()
                .filter(|(at, _)| at == checkpoint.execution_boundary());
            if fault.as_ref().is_none_or(|(_, after)| *after) {
                let bytes = serde_json::to_vec(checkpoint).unwrap();
                self.checkpoints
                    .lock()
                    .unwrap()
                    .push(RunnerCheckpoint::decode(&bytes).unwrap());
            }
            if fault.is_some() {
                return Err(Error::new(
                    ErrorCategory::Host,
                    "injected persistence failure",
                ));
            }
            Ok(())
        })
    }
}
#[derive(Default)]
struct HostImpl;
impl Host for HostImpl {
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
struct ModelImpl {
    responses: Mutex<VecDeque<ModelResponse>>,
    calls: AtomicUsize,
    fail: bool,
}
impl Model for ModelImpl {
    fn provider(&self) -> &str {
        "test"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(Error::new(ErrorCategory::Provider, "ambiguous failure"));
            }
            Ok(self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected model replay"))
        })
    }
}
struct ToolImpl {
    definition: ToolDefinition,
    keys: Mutex<Vec<String>>,
    fail: bool,
}
impl Tool for ToolImpl {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            self.keys
                .lock()
                .unwrap()
                .push(context.idempotency_key.clone().unwrap());
            if self.fail {
                return Err(Error::new(ErrorCategory::Tool, "outcome unknown"));
            }
            Ok(ToolOutput {
                content: vec![Content::Text {
                    text: "done".into(),
                }],
                is_error: false,
                should_pause: false,
            })
        })
    }
}
fn message(role: Role, text: &str) -> RunItem {
    RunItem::Message {
        message: Message {
            role,
            content: vec![Content::Text { text: text.into() }],
        },
    }
}
fn response(items: Vec<RunItem>) -> ModelResponse {
    ModelResponse {
        items,
        usage: Usage {
            input_tokens: 10,
            output_tokens: 2,
            ..Default::default()
        },
        end_turn: None,
        response_id: None,
        metadata: Default::default(),
    }
}
fn call(id: &str) -> RunItem {
    RunItem::ToolCall {
        call: ToolCall {
            id: id.into(),
            name: "effect".into(),
            arguments: json!({}),
        },
    }
}
fn context() -> Context {
    Context {
        run_id: "run-1".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn request(resume: bool) -> RunRequest {
    RunRequest {
        input: if resume {
            vec![]
        } else {
            vec![message(Role::User, "go")]
        },
        policy: RunPolicy::default(),
    }
}
fn setup(
    items: Vec<RunItem>,
    tool_fail: bool,
    model_fail: bool,
) -> (Runner, Arc<ModelImpl>, Arc<ToolImpl>) {
    let model = Arc::new(ModelImpl {
        responses: Mutex::new(VecDeque::from([
            response(items),
            response(vec![message(Role::Assistant, "answer")]),
        ])),
        calls: AtomicUsize::new(0),
        fail: model_fail,
    });
    let tool = Arc::new(ToolImpl {
        definition: ToolDefinition {
            name: "effect".into(),
            description: "test".into(),
            input_schema: schemars::json_schema!({"type":"object"}),
            read_only: true,
            requires_approval: false,
        },
        keys: Mutex::new(vec![]),
        fail: tool_fail,
    });
    let mut agent = AgentConfig::new("agent", ModelBinding::complete("model", model.clone()));
    agent.tools.push(tool.clone());
    let config = RunnerConfig {
        retry: RetryPolicy {
            max_retries: 5,
            ..Default::default()
        },
        ..Default::default()
    };
    (Runner::new(agent, config).unwrap(), model, tool)
}
fn durable(store: Arc<Store>, resume: Option<RunnerCheckpoint>) -> DurableRun {
    DurableRun {
        resume,
        ..DurableRun::new(store)
    }
}
async fn run(
    runner: &Runner,
    store: Arc<Store>,
    checkpoint: Option<RunnerCheckpoint>,
) -> Result<RunOutcome, RunError> {
    runner
        .run_durable(
            context(),
            request(checkpoint.is_some()),
            Arc::new(HostImpl),
            durable(store, checkpoint),
        )
        .await
}

#[tokio::test]
async fn persistence_faults_before_and_after_commit_never_cross_boundary() {
    for boundary in [
        "model_prepared",
        "model_dispatched",
        "model_completed",
        "tool_prepared",
        "tool_dispatched",
        "tool_completed",
    ] {
        for after_commit in [false, true] {
            let (runner, model, tool) = setup(vec![call("one"), call("two")], false, false);
            let store = Arc::new(Store::default());
            *store.fail.lock().unwrap() = Some((boundary.into(), after_commit));
            let error = run(&runner, store.clone(), None).await.err().unwrap();
            assert_eq!(
                error.error.info.message, "injected persistence failure",
                "{boundary}"
            );
            let expected_models = if matches!(boundary, "model_prepared" | "model_dispatched") {
                0
            } else {
                1
            };
            assert_eq!(
                model.calls.load(Ordering::SeqCst),
                expected_models,
                "{boundary}"
            );
            assert_eq!(
                tool.keys.lock().unwrap().len(),
                usize::from(boundary == "tool_completed"),
                "{boundary}"
            );
            let latest = store.latest();
            if latest
                .effect
                .as_ref()
                .is_some_and(|e| matches!(e.state, adk_durable::EffectState::Dispatched))
            {
                *store.fail.lock().unwrap() = None;
                let error = run(&runner, store.clone(), Some(latest))
                    .await
                    .err()
                    .unwrap();
                assert!(error.error.info.message.contains("operator_resolution"));
                assert_eq!(model.calls.load(Ordering::SeqCst), expected_models);
                assert_eq!(
                    tool.keys.lock().unwrap().len(),
                    usize::from(boundary == "tool_completed")
                );
            }
        }
    }
}

#[tokio::test]
async fn completed_model_and_tool_boundaries_resume_remaining_work_once() {
    let (runner, _, _) = setup(vec![call("one"), call("two")], false, false);
    let store = Arc::new(Store::default());
    run(&runner, store.clone(), None).await.unwrap();
    for (boundary, tools_left) in [("model_completed", 2), ("tool_completed", 1)] {
        let checkpoint = store.at(boundary);
        let (resumer, model, tool) = setup(vec![message(Role::Assistant, "resumed")], false, false);
        let resumed_store = Arc::new(Store::default());
        let result = run(&resumer, resumed_store.clone(), Some(checkpoint))
            .await
            .unwrap();
        assert_eq!(tool.keys.lock().unwrap().len(), tools_left);
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
        assert_eq!(result.result.usage.input_tokens, 20);
        let saved = resumed_store.latest();
        assert_eq!(saved.runtime.as_ref().unwrap().turns(), 2);
        assert_eq!(saved.runtime.as_ref().unwrap().tool_calls(), 2);
        assert_eq!(
            saved.runtime.as_ref().unwrap().result().usage,
            result.result.usage
        );
    }
}

#[tokio::test]
async fn prepared_recovery_keeps_effect_key_step_id_and_advances_sequence() {
    let (runner, _, _) = setup(vec![call("one")], false, false);
    let store = Arc::new(Store::default());
    *store.fail.lock().unwrap() = Some(("tool_dispatched".into(), false));
    run(&runner, store.clone(), None).await.err().unwrap();
    let prepared = store.latest();
    assert_eq!(prepared.execution_boundary(), "tool_prepared");
    let (resumer, _, tool) = setup(vec![message(Role::Assistant, "done")], false, false);
    let resumed = Arc::new(Store::default());
    run(&resumer, resumed.clone(), Some(prepared.clone()))
        .await
        .unwrap();
    assert_eq!(
        tool.keys.lock().unwrap()[0],
        prepared.effect.as_ref().unwrap().idempotency_key
    );
    let dispatch = resumed.at("tool_dispatched");
    assert_eq!(dispatch.step_id, prepared.step_id);
    assert_eq!(
        dispatch.effect.as_ref().unwrap().id,
        prepared.effect.as_ref().unwrap().id
    );
    assert!(dispatch.sequence > prepared.sequence);
    assert_ne!(dispatch.attempt_id, prepared.attempt_id);
}

#[tokio::test]
async fn model_and_tool_errors_are_not_automatically_retried() {
    for model_failure in [true, false] {
        let (runner, model, tool) = setup(vec![call("one")], !model_failure, model_failure);
        let store = Arc::new(Store::default());
        assert!(run(&runner, store.clone(), None).await.is_err());
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
        assert_eq!(tool.keys.lock().unwrap().len(), usize::from(!model_failure));
        let mut checkpoint = store.latest();
        checkpoint.effect.as_mut().unwrap().state = adk_durable::EffectState::OutcomeUnknown;
        let error = run(&runner, store, Some(checkpoint)).await.err().unwrap();
        assert!(error.error.info.message.contains("operator_resolution"));
    }
}

#[tokio::test]
async fn terminal_recovery_never_reexecutes_or_writes() {
    let (runner, model, _) = setup(vec![message(Role::Assistant, "done")], false, false);
    let store = Arc::new(Store::default());
    let first = run(&runner, store.clone(), None).await.unwrap();
    let restored = run(&runner, store.clone(), Some(store.latest()))
        .await
        .unwrap();
    assert_eq!(restored.result, first.result);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.checkpoints.lock().unwrap().len(), 5);
}

#[tokio::test]
async fn restart_does_not_reset_deadline_or_turn_budget() {
    let (runner, model, _) = setup(vec![message(Role::Assistant, "done")], false, false);
    let store = Arc::new(Store::default());
    let mut ctx = context();
    ctx.deadline = Some(Instant::now() + Duration::from_millis(100));
    *store.fail.lock().unwrap() = Some(("model_dispatched".into(), false));
    runner
        .run_durable(
            ctx,
            request(false),
            Arc::new(HostImpl),
            durable(store.clone(), None),
        )
        .await
        .err()
        .unwrap();
    let cp = store.latest();
    tokio::time::sleep(Duration::from_millis(120)).await;
    let error = run(&runner, Arc::new(Store::default()), Some(cp))
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::DeadlineExceeded);
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
}

fn verified() -> GoRecovery {
    GoRecovery {
        policy: RunPolicy::default(),
        stop_gate_blocks: None,
        effective_max_turns: None,
        turns: 2,
        usage: Usage {
            input_tokens: 11,
            output_tokens: 7,
            ..Default::default()
        },
        cost: 0.25,
        tool_calls: 1,
        started_at: chrono::Utc::now(),
        deadline_at: None,
        final_output: None,
    }
}
#[tokio::test]
async fn actual_go_fixture_requires_explicit_migration_and_preserves_counters() {
    let cp = RunnerCheckpoint::decode(include_bytes!("fixtures/go-checkpoint.json")).unwrap();
    let (runner, model, _) = setup(vec![message(Role::Assistant, "done")], false, false);
    let store = Arc::new(Store::default());
    let error = run(&runner, store.clone(), Some(cp.clone()))
        .await
        .err()
        .unwrap();
    assert!(error.error.info.message.contains("requires migration"));
    let migrated = runner
        .migrate_go_checkpoint(cp.clone(), verified())
        .unwrap();
    let result = run(&runner, store.clone(), Some(migrated)).await.unwrap();
    assert_eq!(result.result.usage.input_tokens, 21);
    assert_eq!(store.latest().runtime.as_ref().unwrap().turns(), 3);
    assert_eq!(store.latest().runtime.as_ref().unwrap().cost(), 0.25);
    assert_eq!(store.latest().runtime.as_ref().unwrap().tool_calls(), 1);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    for boundary in [
        "model_prepared",
        "tool_prepared",
        "model_completed",
        "approval_pending",
    ] {
        let mut unsafe_cp = cp.clone();
        unsafe_cp.boundary = boundary.into();
        assert!(runner.migrate_go_checkpoint(unsafe_cp, verified()).is_err());
    }
    let mut completed = cp;
    completed.boundary = "run_completed".into();
    let mut recovery = verified();
    recovery.final_output = Some(json!("verified Go result"));
    let migrated = runner.migrate_go_checkpoint(completed, recovery).unwrap();
    let result = run(&runner, store, Some(migrated)).await.unwrap();
    assert_eq!(
        result.result.final_output,
        Some(json!("verified Go result"))
    );
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn future_schemas_and_changed_policy_fail_closed_and_go_reader_is_gated() {
    let (runner, _, _) = setup(vec![message(Role::Assistant, "done")], false, false);
    let store = Arc::new(Store::default());
    run(&runner, store.clone(), None).await.unwrap();
    let cp = store.at("model_dispatched");
    assert_eq!(cp.boundary, "model_completed"); // Baseline Go refuses this boundary.
    let mut value = serde_json::to_value(&cp).unwrap();
    value["schema_version"] = json!(2);
    assert!(RunnerCheckpoint::decode(&serde_json::to_vec(&value).unwrap()).is_err());
    value["schema_version"] = json!(1);
    value["runtime"]["version"] = json!(2);
    assert!(RunnerCheckpoint::decode(&serde_json::to_vec(&value).unwrap()).is_err());
    let cp = store.at("model_prepared");
    let mut req = request(true);
    req.policy.tools.access = AccessMode::FullAccess;
    let error = runner
        .run_durable(context(), req, Arc::new(HostImpl), durable(store, Some(cp)))
        .await
        .err()
        .unwrap();
    assert!(error.error.info.message.contains("security policy changed"));
}

#[tokio::test]
async fn real_filesystem_adapter_commits_go_state_effects_events_and_counters() {
    use adk_durable::{FilesystemStore, RunId, RunSnapshot, RunStore, TenantId};
    let directory = tempfile::tempdir().unwrap();
    let fs = Arc::new(FilesystemStore::new(directory.path(), Default::default()).unwrap());
    let tenant = TenantId::from("tenant");
    let run_id = RunId::from("run-1");
    fs.create(RunSnapshot::new(
        tenant.clone(),
        run_id.clone(),
        chrono::Utc::now(),
    ))
    .unwrap();
    let lease = fs
        .acquire_lease(&tenant, &run_id, "worker", Duration::from_secs(60))
        .unwrap();
    let adapter = Arc::new(StoredCheckpointStore::open(fs.clone(), lease.clone()).unwrap());
    let (runner, model, tool) = setup(vec![call("one")], false, false);
    let result = runner
        .run_durable(
            context(),
            request(false),
            Arc::new(HostImpl),
            DurableRun::new(adapter.clone()),
        )
        .await
        .unwrap();
    let (snapshot, events) = fs.load(&tenant, &run_id).unwrap();
    assert_eq!(snapshot.cumulative_budget.input_tokens, 20);
    assert_eq!(snapshot.cumulative_budget.tool_calls, 1);
    assert_eq!(snapshot.effects.len(), 3);
    assert!(
        snapshot
            .effects
            .iter()
            .all(|e| e.state == adk_durable::EffectState::Succeeded)
    );
    assert_eq!(snapshot.revision as usize, events.len());
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type == "tool_dispatched")
            .count(),
        1
    );
    fs.release_lease(&lease).unwrap();
    let lease = fs
        .acquire_lease(&tenant, &run_id, "new-worker", Duration::from_secs(60))
        .unwrap();
    let reopened = Arc::new(StoredCheckpointStore::open(fs.clone(), lease).unwrap());
    let durable = DurableRun {
        resume: reopened.checkpoint().unwrap(),
        ..DurableRun::new(reopened)
    };
    let restored = runner
        .run_durable(context(), request(true), Arc::new(HostImpl), durable)
        .await
        .unwrap();
    assert_eq!(restored.result, result.result);
    assert_eq!(model.calls.load(Ordering::SeqCst), 2);
    assert_eq!(tool.keys.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn failed_approval_checkpoint_prevents_callback() {
    struct ApprovalHost(AtomicUsize);
    impl Host for ApprovalHost {
        fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
            Box::pin(async { Ok(()) })
        }
        fn approve<'a>(
            &'a self,
            _: &'a Context,
            _: ApprovalRequest,
        ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(ApprovalDecision::Approve) })
        }
    }
    let (runner, model, tool) = setup(vec![call("one")], false, false);
    let store = Arc::new(Store::default());
    *store.fail.lock().unwrap() = Some(("approval_pending".into(), false));
    let host = Arc::new(ApprovalHost(AtomicUsize::new(0)));
    let mut req = request(false);
    req.policy.tools.approval = ApprovalPolicy::All;
    let result = runner
        .run_durable(context(), req, host.clone(), DurableRun::new(store))
        .await;
    assert!(result.is_err());
    assert_eq!(host.0.load(Ordering::SeqCst), 0);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert!(tool.keys.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cumulative_turn_token_and_cost_limits_remain_exhausted_after_go_migration() {
    struct Cost;
    impl CostEstimator for Cost {
        fn cost(&self, _: &str, _: &Usage) -> f64 {
            0.1
        }
    }
    for budget in ["turn", "token", "cost"] {
        let model = Arc::new(ModelImpl {
            responses: Mutex::new(VecDeque::new()),
            calls: AtomicUsize::new(0),
            fail: false,
        });
        let agent = AgentConfig::new("agent", ModelBinding::complete("model", model.clone()));
        let mut config = RunnerConfig::default();
        if budget == "token" {
            config.limits.max_tokens = Some(18);
        }
        if budget == "cost" {
            config.limits.max_cost = Some(0.25);
            config.cost_estimator = Some(Arc::new(Cost));
        }
        let runner = Runner::new(agent, config).unwrap();
        let cp = RunnerCheckpoint::decode(include_bytes!("fixtures/go-checkpoint.json")).unwrap();
        let mut verified = verified();
        if budget == "turn" {
            verified.policy.max_turns = std::num::NonZeroU32::new(2).unwrap();
        }
        let policy = verified.policy.clone();
        let cp = runner.migrate_go_checkpoint(cp, verified).unwrap();
        let error = runner
            .run_durable(
                context(),
                RunRequest {
                    input: vec![],
                    policy,
                },
                Arc::new(HostImpl),
                durable(Arc::new(Store::default()), Some(cp)),
            )
            .await
            .err()
            .unwrap();
        assert_eq!(
            error.error.info.category,
            if budget == "turn" {
                ErrorCategory::MaxTurns
            } else {
                ErrorCategory::Guardrail
            }
        );
        assert_eq!(model.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn cancellation_during_async_persistence_never_dispatches() {
    struct PendingStore(tokio::sync::Notify);
    impl CheckpointStore for PendingStore {
        fn persist<'a>(
            &'a self,
            _: &'a Context,
            cp: &'a RunnerCheckpoint,
        ) -> BoxFuture<'a, Result<(), Error>> {
            Box::pin(async move {
                if cp.execution_boundary() == "model_dispatched" {
                    self.0.notify_one();
                    std::future::pending::<()>().await;
                }
                Ok(())
            })
        }
    }
    let (runner, model, _) = setup(vec![message(Role::Assistant, "never")], false, false);
    let store = Arc::new(PendingStore(tokio::sync::Notify::new()));
    let cancel = Arc::new(CancellationToken::new());
    let mut ctx = context();
    ctx.cancellation = cancel.clone();
    let (outcome, ()) = tokio::join!(
        runner.run_durable(
            ctx,
            request(false),
            Arc::new(HostImpl),
            DurableRun::new(store.clone())
        ),
        async {
            store.0.notified().await;
            cancel.cancel();
        }
    );
    assert_eq!(
        outcome.err().unwrap().error.info.category,
        ErrorCategory::Cancelled
    );
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn destructive_redaction_never_acknowledges_executable_state() {
    use adk_durable::{FilesystemStore, RunId, RunSnapshot, RunStore, StoreOptions, TenantId};
    let directory = tempfile::tempdir().unwrap();
    let fs = Arc::new(
        FilesystemStore::new(
            directory.path(),
            StoreOptions {
                redactor: Some(Arc::new(|_, _: serde_json::Value| {
                    Ok(serde_json::Value::Null)
                })),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let tenant = TenantId::from("tenant");
    let run_id = RunId::from("run-1");
    fs.create(RunSnapshot::new(
        tenant.clone(),
        run_id.clone(),
        chrono::Utc::now(),
    ))
    .unwrap();
    let lease = fs
        .acquire_lease(&tenant, &run_id, "worker", Duration::from_secs(60))
        .unwrap();
    let adapter = Arc::new(StoredCheckpointStore::open(fs, lease).unwrap());
    let (runner, model, _) = setup(vec![message(Role::Assistant, "never")], false, false);
    for _ in 0..2 {
        let error = runner
            .run_durable(
                context(),
                request(false),
                Arc::new(HostImpl),
                DurableRun::new(adapter.clone()),
            )
            .await
            .err()
            .unwrap();
        assert_eq!(error.error.info.category, ErrorCategory::Host);
    }
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn go_safe_boundaries_and_terminal_children_resume_without_replaying_history() {
    for boundary in [
        "tool_completed",
        "handoff_completed",
        "paused",
        "child_changed",
    ] {
        let mut cp =
            RunnerCheckpoint::decode(include_bytes!("fixtures/go-checkpoint.json")).unwrap();
        cp.boundary = boundary.into();
        cp.children = Some(
            json!({"records":[{"task":{"id":"c1","status":"completed"}},{"task":{"id":"c2","status":"failed"}},{"task":{"id":"c3","status":"cancelled"}}]}),
        );
        let (runner, model, tool) = setup(vec![message(Role::Assistant, "done")], false, false);
        let migrated = runner
            .migrate_go_checkpoint(cp.clone(), verified())
            .unwrap();
        let result = run(&runner, Arc::new(Store::default()), Some(migrated))
            .await
            .unwrap();
        assert_eq!(result.result.status, RunStatus::Completed);
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
        assert!(tool.keys.lock().unwrap().is_empty());
        cp.children = Some(json!({"records":[{"task":{"id":"c4","status":"running"}}]}));
        let migrated = runner.migrate_go_checkpoint(cp, verified()).unwrap();
        assert!(
            run(&runner, Arc::new(Store::default()), Some(migrated))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn durable_stream_is_lazy_and_persists_completion() {
    let (runner, model, _) = setup(vec![message(Role::Assistant, "done")], false, false);
    let store = Arc::new(Store::default());
    let stream = runner.stream_durable(
        context(),
        request(false),
        Arc::new(HostImpl),
        DurableRun::new(store.clone()),
    );
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
    assert!(store.checkpoints.lock().unwrap().is_empty());
    let result = stream.finish().await.unwrap();
    assert_eq!(result.result.status, RunStatus::Completed);
    assert_eq!(store.latest().execution_boundary(), "run_completed");
}

struct DeferredHost(AtomicUsize);
impl Host for DeferredHost {
    fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ApprovalDecision::Defer)
        })
    }
}
#[tokio::test]
async fn approval_pause_survives_restart_and_completed_approved_effect_is_not_replayed() {
    let (runner, model, tool) = setup(vec![call("one")], false, false);
    let store = Arc::new(Store::default());
    let host = Arc::new(DeferredHost(AtomicUsize::new(0)));
    let mut req = request(false);
    req.policy.tools.approval = ApprovalPolicy::All;
    let result = runner
        .run_durable(
            context(),
            req.clone(),
            host.clone(),
            durable(store.clone(), None),
        )
        .await
        .unwrap();
    assert_eq!(result.result.status, RunStatus::Paused);
    drop(result);
    let checkpoint = store.latest();
    req.input.clear();
    let restored = runner
        .run_durable(
            context(),
            req.clone(),
            host.clone(),
            durable(store.clone(), Some(checkpoint)),
        )
        .await
        .unwrap();
    assert_eq!(host.0.load(Ordering::SeqCst), 1);
    assert!(tool.keys.lock().unwrap().is_empty());
    let completed = restored
        .continuation
        .unwrap()
        .resume(Some(ApprovalDecision::Approve))
        .await
        .unwrap();
    assert_eq!(completed.result.status, RunStatus::Completed);
    assert_eq!(tool.keys.lock().unwrap().len(), 1);
    let checkpoint = store.at("tool_completed");
    model
        .responses
        .lock()
        .unwrap()
        .push_back(response(vec![message(Role::Assistant, "after recovery")]));
    runner
        .run_durable(
            context(),
            req,
            host.clone(),
            durable(Arc::new(Store::default()), Some(checkpoint)),
        )
        .await
        .unwrap();
    assert_eq!(host.0.load(Ordering::SeqCst), 1);
    assert_eq!(tool.keys.lock().unwrap().len(), 1);
}

#[derive(Default)]
struct Children(Mutex<Option<serde_json::Value>>);
impl ChildCheckpointOwner for Children {
    fn restore<'a>(
        &'a self,
        _: &'a Context,
        checkpoint: serde_json::Value,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            *self.0.lock().unwrap() = Some(checkpoint);
            Ok(())
        })
    }
    fn checkpoint<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<serde_json::Value, Error>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(json!({"records":[]})))
        })
    }
}
#[tokio::test]
async fn child_owner_restores_active_records_as_reconciling_and_retains_delivery_state() {
    let (runner, _, _) = setup(vec![message(Role::Assistant, "done")], false, false);
    let mut cp = RunnerCheckpoint::decode(include_bytes!("fixtures/go-checkpoint.json")).unwrap();
    cp.children = Some(
        json!({"records":[{"task":{"id":"child","status":"running","waiting_on":["a"]},"result_delivered":true,"security_baseline":{"tool_access_level":"read_only"},"queued_messages":[{"type": "message", "message_text":"queued"}]}]}),
    );
    let migrated = runner.migrate_go_checkpoint(cp, verified()).unwrap();
    let children = Arc::new(Children::default());
    let store = Arc::new(Store::default());
    let mut d = durable(store.clone(), Some(migrated));
    d.children = Some(children.clone());
    runner
        .run_durable(context(), request(true), Arc::new(HostImpl), d)
        .await
        .unwrap();
    let saved = store.latest().children.unwrap();
    assert_eq!(saved["records"][0]["task"]["status"], "reconciling");
    assert_eq!(saved["records"][0]["result_delivered"], true);
    assert_eq!(
        saved["records"][0]["queued_messages"][0]["message_text"],
        "queued"
    );
    assert_eq!(saved, children.0.lock().unwrap().clone().unwrap());
}

#[tokio::test]
async fn go_observational_callbacks_are_allowed_in_durable_execution() {
    struct Callbacks(AtomicUsize);
    impl compat::GoLifecycleCallbacks for Callbacks {
        fn on_model_start(&self, _: &Context, _: &str, _: &str, _: u32) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let (_, model, _) = setup(vec![message(Role::Assistant, "done")], false, false);
    let agent = AgentConfig::new("agent", ModelBinding::complete("model", model));
    let callbacks = Arc::new(Callbacks(AtomicUsize::new(0)));
    let runner = Runner::new(
        agent,
        RunnerConfig {
            hooks: Some(Arc::new(compat::GoCallbackAdapter::new(callbacks.clone()))),
            ..Default::default()
        },
    )
    .unwrap();
    run(&runner, Arc::new(Store::default()), None)
        .await
        .unwrap();
    assert_eq!(callbacks.0.load(Ordering::SeqCst), 1);
}

struct ScriptStream(VecDeque<ModelEvent>);
impl ModelStream for ScriptStream {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async move { Ok(self.0.pop_front()) })
    }
}
impl StreamingModel for ModelImpl {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let response = self.responses.lock().unwrap().pop_front().unwrap();
            Ok(Box::new(ScriptStream(VecDeque::from([
                ModelEvent::TextDelta {
                    delta: "partial".into(),
                },
                ModelEvent::Complete { response },
            ]))) as Box<dyn ModelStream>)
        })
    }
}
#[tokio::test]
async fn dropping_genuine_durable_stream_never_replays_uncertain_model_dispatch() {
    let (_, model, _) = setup(vec![message(Role::Assistant, "done")], false, false);
    let runner = Runner::new(
        AgentConfig::new("agent", ModelBinding::streaming("model", model.clone())),
        RunnerConfig::default(),
    )
    .unwrap();
    let store = Arc::new(Store::default());
    let mut stream = runner.stream_durable(
        context(),
        request(false),
        Arc::new(HostImpl),
        DurableRun::new(store.clone()),
    );
    while let Some(event) = stream.next().await {
        if matches!(
            event,
            RunEvent::Model {
                event: ModelEvent::TextDelta { .. }
            }
        ) {
            break;
        }
    }
    assert_eq!(store.latest().execution_boundary(), "model_dispatched");
    drop(stream);
    let error = run(&runner, Arc::new(Store::default()), Some(store.latest()))
        .await
        .err()
        .unwrap();
    assert!(error.error.info.message.contains("operator_resolution"));
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    let store = Arc::new(Store::default());
    runner
        .stream_durable(
            context(),
            request(false),
            Arc::new(HostImpl),
            DurableRun::new(store.clone()),
        )
        .finish()
        .await
        .unwrap();
    assert_eq!(store.latest().execution_boundary(), "run_completed");
}

#[tokio::test]
async fn actual_go_emitted_completed_boundaries_resume_without_replaying_effects() {
    let cases: std::collections::BTreeMap<String, Vec<RunnerCheckpoint>> =
        serde_json::from_str(include_str!("fixtures/go-boundaries.json")).unwrap();
    for (mode, checkpoints) in cases {
        for cp in checkpoints {
            let (_, model, tool) = setup(vec![message(Role::Assistant, "resumed")], false, false);
            let mut agent =
                AgentConfig::new("agent", ModelBinding::complete("model", model.clone()));
            agent.tools.push(tool.clone());
            agent.handoffs.push(Handoff {
                definition: ToolDefinition {
                    name: "transfer_to_target".into(),
                    description: "".into(),
                    input_schema: schemars::json_schema!({"type":"object"}),
                    read_only: true,
                    requires_approval: false,
                },
                target: Arc::new(AgentConfig::new(
                    "target",
                    ModelBinding::complete("model", model.clone()),
                )),
            });
            let runner = Runner::new(agent, RunnerConfig::default()).unwrap();
            let supported = matches!(
                cp.boundary.as_str(),
                "run_started" | "tool_completed" | "handoff_completed" | "paused" | "run_completed"
            ) && !(mode == "approval" && cp.boundary == "tool_completed");
            let mut recovery = verified();
            recovery.turns = 4;
            if cp.boundary == "run_completed" {
                recovery.final_output = Some(json!("done"));
            }
            let migrated = runner.migrate_go_checkpoint(cp.clone(), recovery);
            assert_eq!(
                migrated.is_ok(),
                supported,
                "{mode}: {}: {migrated:?}",
                cp.boundary
            );
            if let Ok(cp) = migrated {
                let terminal = cp.execution_boundary() == "run_completed";
                let result = run(&runner, Arc::new(Store::default()), Some(cp))
                    .await
                    .unwrap();
                assert_eq!(result.result.status, RunStatus::Completed);
                assert!(tool.keys.lock().unwrap().is_empty());
                assert_eq!(model.calls.load(Ordering::SeqCst), usize::from(!terminal));
            }
        }
    }
}

#[tokio::test]
async fn native_handoff_checkpoint_restores_target_and_pairs_go_history() {
    let (_, model, _) = setup(
        vec![RunItem::ToolCall {
            call: ToolCall {
                id: "handoff-1".into(),
                name: "transfer".into(),
                arguments: json!({}),
            },
        }],
        false,
        false,
    );
    let mut agent = AgentConfig::new("agent", ModelBinding::complete("model", model.clone()));
    agent.handoffs.push(Handoff {
        definition: ToolDefinition {
            name: "transfer".into(),
            description: "".into(),
            input_schema: schemars::json_schema!({"type":"object"}),
            read_only: true,
            requires_approval: false,
        },
        target: Arc::new(AgentConfig::new(
            "target",
            ModelBinding::complete("model", model.clone()),
        )),
    });
    let runner = Runner::new(agent, RunnerConfig::default()).unwrap();
    let store = Arc::new(Store::default());
    *store.fail.lock().unwrap() = Some(("handoff_completed".into(), true));
    assert!(run(&runner, store.clone(), None).await.is_err());
    let cp = store.latest();
    assert_eq!(cp.agent_name, "target");
    assert_eq!(
        cp.history
            .last()
            .unwrap()
            .tool_output
            .as_ref()
            .unwrap()
            .call_id,
        "handoff-1"
    );
    let result = run(&runner, Arc::new(Store::default()), Some(cp))
        .await
        .unwrap();
    assert_eq!(result.result.last_agent.as_deref(), Some("target"));
    assert_eq!(model.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn go_approval_gate_cannot_reset_durable_turn_budget() {
    struct Gate;
    impl compat::GoApprovalGate for Gate {
        fn approve<'a>(
            &'a self,
            _: &'a Context,
            _: &'a ApprovalRequest,
        ) -> BoxFuture<'a, Result<compat::GoApprovalDecision, Error>> {
            Box::pin(async {
                Ok(compat::GoApprovalDecision {
                    approved: true,
                    reason: String::new(),
                })
            })
        }
    }
    let (runner, model, tool) = setup(vec![call("one")], false, false);
    let mut req = request(false);
    req.policy.max_turns = std::num::NonZeroU32::new(1).unwrap();
    req.policy.tools.approval = ApprovalPolicy::All;
    let store = Arc::new(Store::default());
    let paused = runner
        .run_durable(
            context(),
            req,
            Arc::new(DeferredHost(AtomicUsize::new(0))),
            DurableRun::new(store.clone()),
        )
        .await
        .unwrap();
    let error = paused
        .continuation
        .unwrap()
        .resume_go_gate(&Gate)
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::MaxTurns);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert_eq!(tool.keys.lock().unwrap().len(), 1);
    assert_eq!(store.latest().runtime.unwrap().turns(), 1);
}

#[tokio::test]
async fn prepared_approved_call_restores_exact_grant_without_another_callback() {
    let (runner, _, tool) = setup(vec![call("one")], false, false);
    let store = Arc::new(Store::default());
    *store.fail.lock().unwrap() = Some(("tool_prepared".into(), true));
    let mut req = request(false);
    req.policy.tools.approval = ApprovalPolicy::All;
    assert!(
        runner
            .run_durable(
                context(),
                req.clone(),
                Arc::new(HostImpl),
                DurableRun::new(store.clone())
            )
            .await
            .is_err()
    );
    assert!(tool.keys.lock().unwrap().is_empty());
    let host = Arc::new(DeferredHost(AtomicUsize::new(0)));
    req.input.clear();
    let result = runner
        .run_durable(
            context(),
            req,
            host.clone(),
            durable(Arc::new(Store::default()), Some(store.latest())),
        )
        .await
        .unwrap();
    assert_eq!(result.result.status, RunStatus::Completed);
    assert_eq!(host.0.load(Ordering::SeqCst), 0);
    assert_eq!(tool.keys.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn actual_go_reader_accepts_rust_approval_history_and_refuses_nonterminal_replay() {
    if std::env::var_os("ADK_TEST_GO").is_none() {
        return;
    }
    let (runner, _, _) = setup(vec![call("one")], false, false);
    let store = Arc::new(Store::default());
    let mut req = request(false);
    req.policy.tools.approval = ApprovalPolicy::All;
    runner
        .run_durable(
            context(),
            req,
            Arc::new(HostImpl),
            DurableRun::new(store.clone()),
        )
        .await
        .unwrap();
    assert!(
        store
            .latest()
            .history
            .iter()
            .any(|item| item.tool_approval.is_some())
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("checkpoints.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&*store.checkpoints.lock().unwrap()).unwrap(),
    )
    .unwrap();
    let output = std::process::Command::new("go")
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../repos/sdk"))
        .args(["run", "../../crates/adk-runtime/tests/fixtures/verify.go"])
        .arg(path)
        .env("GOTOOLCHAIN", "local")
        .env("GOTELEMETRY", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
}

#[tokio::test]
async fn child_restore_snapshot_and_unknown_status_fail_before_parent_dispatch() {
    struct FailedChildren;
    impl ChildCheckpointOwner for FailedChildren {
        fn restore<'a>(
            &'a self,
            _: &'a Context,
            _: serde_json::Value,
        ) -> BoxFuture<'a, Result<(), Error>> {
            Box::pin(async { Err(Error::new(ErrorCategory::Host, "restore failed")) })
        }
        fn checkpoint<'a>(
            &'a self,
            _: &'a Context,
        ) -> BoxFuture<'a, Result<serde_json::Value, Error>> {
            Box::pin(async { Err(Error::new(ErrorCategory::Host, "snapshot failed")) })
        }
    }
    let (runner, model, _) = setup(vec![message(Role::Assistant, "done")], false, false);
    let mut d = DurableRun::new(Arc::new(Store::default()));
    d.children = Some(Arc::new(FailedChildren));
    assert!(
        runner
            .run_durable(context(), request(false), Arc::new(HostImpl), d)
            .await
            .is_err()
    );
    let mut cp = RunnerCheckpoint::decode(include_bytes!("fixtures/go-checkpoint.json")).unwrap();
    cp.children = Some(json!({"records":[{"task":{"id":"c","status":"running"}}]}));
    let migrated = runner
        .migrate_go_checkpoint(cp.clone(), verified())
        .unwrap();
    let mut d = durable(Arc::new(Store::default()), Some(migrated));
    d.children = Some(Arc::new(FailedChildren));
    assert!(
        runner
            .run_durable(context(), request(true), Arc::new(HostImpl), d)
            .await
            .is_err()
    );
    cp.children.as_mut().unwrap()["records"][0]["task"]["status"] = json!("future_status");
    assert!(runner.migrate_go_checkpoint(cp, verified()).is_err());
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn native_tool_pause_restarts_after_completed_tool_without_dispatching_it_again() {
    struct PausingTool(Arc<ToolImpl>);
    impl Tool for PausingTool {
        fn definition(&self) -> &ToolDefinition {
            self.0.definition()
        }
        fn execute<'a>(
            &'a self,
            context: &'a ToolContext,
            call: ToolCall,
        ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
            Box::pin(async move {
                let mut output = self.0.execute(context, call).await?;
                output.should_pause = true;
                Ok(output)
            })
        }
    }
    let (_, model, tool) = setup(vec![call("one")], false, false);
    let mut agent = AgentConfig::new("agent", ModelBinding::complete("model", model.clone()));
    agent.tools.push(Arc::new(PausingTool(tool.clone())));
    let runner = Runner::new(agent, RunnerConfig::default()).unwrap();
    let store = Arc::new(Store::default());
    let outcome = run(&runner, store.clone(), None).await.unwrap();
    assert_eq!(outcome.result.status, RunStatus::Paused);
    drop(outcome);
    let cp = store.latest();
    assert_eq!(cp.execution_boundary(), "paused");
    let outcome = run(&runner, Arc::new(Store::default()), Some(cp))
        .await
        .unwrap();
    assert_eq!(outcome.result.status, RunStatus::Completed);
    assert_eq!(tool.keys.lock().unwrap().len(), 1);
    assert_eq!(model.calls.load(Ordering::SeqCst), 2);
}

struct ReplaySafeGate {
    key: Option<&'static str>,
    calls: AtomicUsize,
}
impl StopGate for ReplaySafeGate {
    fn durable_key(&self) -> Option<&str> {
        self.key
    }
    fn check<'a>(
        &'a self,
        _: &'a Context,
        _: &'a serde_json::Value,
    ) -> BoxFuture<'a, Result<Option<String>, Error>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some(String::new()))
        })
    }
}
fn gated_runner(
    model: Arc<ModelImpl>,
    tool: Arc<ToolImpl>,
    gate: Arc<ReplaySafeGate>,
    cap: usize,
) -> Runner {
    let mut agent = AgentConfig::new("agent", ModelBinding::complete("model", model));
    agent.tools.push(tool);
    Runner::new(
        agent,
        RunnerConfig {
            stop_gate: Some(gate),
            stop_gate_max_blocks: cap,
            ..Default::default()
        },
    )
    .unwrap()
}

#[tokio::test]
async fn durable_stop_gate_matches_go_across_crashes_and_tool_progress() {
    let baseline: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/go-stop-gate.json")).unwrap();
    for name in ["cap", "tool_reset"] {
        let expected = &baseline[name];
        let (_, model, tool) = setup(vec![], false, false);
        let candidate = response(vec![message(Role::Assistant, "candidate")]);
        *model.responses.lock().unwrap() = if name == "cap" {
            VecDeque::from([candidate.clone(), candidate.clone(), candidate])
        } else {
            VecDeque::from([
                candidate.clone(),
                response(vec![call("one")]),
                candidate.clone(),
                candidate.clone(),
                candidate,
            ])
        };
        let gate = Arc::new(ReplaySafeGate {
            key: Some("always-block-v1"),
            calls: AtomicUsize::new(0),
        });
        let runner = gated_runner(model.clone(), tool.clone(), gate.clone(), 2);
        let store = Arc::new(Store::default());
        *store.fail.lock().unwrap() = Some(("model_completed".into(), true));
        let mut req = request(false);
        req.policy.max_turns =
            std::num::NonZeroU32::new(expected["max_turns"].as_u64().unwrap() as u32).unwrap();
        let mut checkpoint = None;
        let mut result = None;
        for iteration in 0..20 {
            match runner
                .run_durable(
                    context(),
                    req.clone(),
                    Arc::new(HostImpl),
                    durable(store.clone(), checkpoint),
                )
                .await
            {
                Ok(outcome) => {
                    result = Some(outcome.result);
                    break;
                }
                Err(error) => assert_eq!(error.error.info.category, ErrorCategory::Host),
            }
            let saved = store.latest();
            let value = serde_json::to_value(&saved).unwrap();
            if iteration == 0 {
                assert_eq!(value["runtime"]["phase"], "Finalize");
                assert_eq!(model.calls.load(Ordering::SeqCst), 1);
                assert_eq!(gate.calls.load(Ordering::SeqCst), 0);
            }
            assert_eq!(value["runtime"]["base_turn_limit"], expected["max_turns"]);
            if name == "cap" && value["runtime"]["phase"] == "Model" {
                assert_eq!(
                    value["runtime"]["policy"]["max_turns"].as_u64().unwrap(),
                    value["runtime"]["stop_gate_blocks"].as_u64().unwrap() + 1
                );
            }
            checkpoint = Some(saved);
            req.input.clear();
        }
        let result = result.expect("gate cap must terminate after recovery");
        let feedback: Vec<_> = result
            .history
            .iter()
            .filter_map(|item| match item {
                RunItem::Message { message } => {
                    message.content.iter().find_map(|content| match content {
                        Content::Text { text }
                            if text.starts_with("[SYSTEM] Final answer blocked") =>
                        {
                            Some(text.clone())
                        }
                        _ => None,
                    })
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            serde_json::to_value(feedback).unwrap(),
            expected["feedback"]
        );
        assert_eq!(
            result.final_output.as_ref().unwrap(),
            &expected["final_output"]
        );
        assert_eq!(
            model.calls.load(Ordering::SeqCst) as u64,
            expected["calls"].as_u64().unwrap()
        );
        assert_eq!(
            gate.calls.load(Ordering::SeqCst) as u64,
            expected["gate_calls"].as_u64().unwrap()
        );
        assert_eq!(
            tool.keys.lock().unwrap().len(),
            usize::from(name == "tool_reset")
        );
    }
}

#[tokio::test]
async fn stop_gate_replay_uses_saved_candidate_and_refuses_changed_configuration() {
    let (_, model, tool) = setup(vec![message(Role::Assistant, "candidate")], false, false);
    model
        .responses
        .lock()
        .unwrap()
        .push_back(response(vec![message(Role::Assistant, "candidate")]));
    let gate = Arc::new(ReplaySafeGate {
        key: Some("v1"),
        calls: AtomicUsize::new(0),
    });
    let runner = gated_runner(model.clone(), tool.clone(), gate.clone(), 2);
    let store = Arc::new(Store::default());
    *store.fail.lock().unwrap() = Some(("model_completed".into(), true));
    assert!(run(&runner, store.clone(), None).await.is_err());
    let candidate = store.latest();
    for (key, cap) in [(Some("v2"), 2), (Some("v1"), 3), (None, 2)] {
        let changed = gated_runner(
            model.clone(),
            tool.clone(),
            Arc::new(ReplaySafeGate {
                key,
                calls: AtomicUsize::new(0),
            }),
            cap,
        );
        assert!(
            run(
                &changed,
                Arc::new(Store::default()),
                Some(candidate.clone())
            )
            .await
            .is_err()
        );
    }
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    *store.fail.lock().unwrap() = Some(("model_completed".into(), false));
    assert!(run(&runner, store.clone(), Some(candidate)).await.is_err());
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert_eq!(gate.calls.load(Ordering::SeqCst), 1);
    *store.fail.lock().unwrap() = None;
    let result = run(&runner, store.clone(), Some(store.latest()))
        .await
        .unwrap();
    assert_eq!(result.result.status, RunStatus::Completed);
    assert_eq!(model.calls.load(Ordering::SeqCst), 3);
    assert_eq!(gate.calls.load(Ordering::SeqCst), 3);
    let cp = RunnerCheckpoint::decode(include_bytes!("fixtures/go-checkpoint.json")).unwrap();
    assert!(
        runner
            .migrate_go_checkpoint(cp.clone(), verified())
            .is_err()
    );
    let mut evidence = verified();
    evidence.stop_gate_blocks = Some(1);
    evidence.effective_max_turns = Some(evidence.policy.max_turns);
    let migrated = runner.migrate_go_checkpoint(cp, evidence).unwrap();
    assert_eq!(
        serde_json::to_value(migrated).unwrap()["runtime"]["stop_gate_blocks"],
        1
    );
}

#[test]
fn actual_go_stop_gate_fixture_is_current() {
    if std::env::var_os("ADK_TEST_GO").is_none() {
        return;
    }
    let output = std::process::Command::new("go")
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../repos/sdk"))
        .args([
            "run",
            "../../crates/adk-runtime/tests/fixtures/boundaries.go",
            "stop-gate",
        ])
        .env("GOTOOLCHAIN", "local")
        .env("GOTELEMETRY", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::from_str::<serde_json::Value>(include_str!("fixtures/go-stop-gate.json"))
            .unwrap()
    );
}

struct DurableGuard {
    key: Option<&'static str>,
    calls: Arc<AtomicUsize>,
}
impl Guardrail for DurableGuard {
    fn name(&self) -> &str {
        "policy"
    }
    fn durable_key(&self) -> Option<&str> {
        self.key
    }
    fn check<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
        _: GuardrailInput<'a>,
    ) -> BoxFuture<'a, Result<Option<GuardrailResult>, Error>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        })
    }
}

#[tokio::test]
async fn durable_guardrails_require_stable_keys_and_completed_recovery_preserves_reports() {
    let calls = Arc::new(AtomicUsize::new(0));
    let make_runner = |key| {
        let model = Arc::new(ModelImpl {
            responses: Mutex::new(VecDeque::from([response(vec![message(
                Role::Assistant,
                "done",
            )])])),
            calls: AtomicUsize::new(0),
            fail: false,
        });
        let mut agent = AgentConfig::new("agent", ModelBinding::complete("model", model));
        agent.input_guardrails.push(Arc::new(DurableGuard {
            key,
            calls: calls.clone(),
        }));
        Runner::new(agent, RunnerConfig::default()).unwrap()
    };
    let store = Arc::new(Store::default());
    let error = run(&make_runner(None), store.clone(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::Unsupported);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(store.checkpoints.lock().unwrap().is_empty());
    let runner = make_runner(Some("v1"));
    let first = run(&runner, store.clone(), None).await.unwrap();
    assert_eq!(first.result.guardrails.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let checkpoint = store.latest();
    let recovered = run(&runner, store.clone(), Some(checkpoint.clone()))
        .await
        .unwrap();
    assert_eq!(recovered.result.guardrails, first.result.guardrails);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        run(&make_runner(Some("v2")), store, Some(checkpoint))
            .await
            .is_err()
    );
}
