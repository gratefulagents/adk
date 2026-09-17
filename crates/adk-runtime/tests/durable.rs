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
        "paused",
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
