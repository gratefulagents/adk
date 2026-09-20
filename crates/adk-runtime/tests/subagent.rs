use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use adk_core::{BoxFuture, Content, Context, Error, ErrorCategory, Message, Role, RunItem};
use adk_runtime::{CancellationToken, ChildCheckpointOwner, subagent::*};
use tokio::sync::{Notify, Semaphore, mpsc, oneshot};

fn context() -> Context {
    Context {
        run_id: "parent".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn config() -> SchedulerConfig {
    SchedulerConfig {
        agents: BTreeMap::from([("worker".into(), SecurityBaseline::default())]),
        ..Default::default()
    }
}
fn request(id: &str) -> Submission {
    let mut request = Submission::new("worker", format!("work on {id}"));
    request.id = id.into();
    request
}
fn text(text: &str) -> RunItem {
    RunItem::Message {
        message: Message {
            role: Role::User,
            content: vec![Content::Text { text: text.into() }],
        },
    }
}

#[derive(Default)]
struct Stats {
    active: AtomicUsize,
    peak: AtomicUsize,
    finished: AtomicUsize,
}
struct Live(Arc<Stats>);
impl Drop for Live {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
        self.0.finished.fetch_add(1, Ordering::SeqCst);
    }
}
struct Started {
    invocation: ChildInvocation,
    control: ChildControl,
    done: oneshot::Sender<ChildOutcome>,
}
struct Executor {
    started: mpsc::UnboundedSender<Started>,
    stats: Arc<Stats>,
}
impl ChildExecutor for Executor {
    fn execute<'a>(
        &'a self,
        invocation: ChildInvocation,
        control: ChildControl,
    ) -> BoxFuture<'a, Result<ChildOutcome, Error>> {
        Box::pin(async move {
            let active = self.stats.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.stats.peak.fetch_max(active, Ordering::SeqCst);
            let _live = Live(self.stats.clone());
            let (done, result) = oneshot::channel();
            self.started
                .send(Started {
                    invocation,
                    control,
                    done,
                })
                .unwrap();
            Ok(result
                .await
                .unwrap_or_else(|_| ChildOutcome::failed("test sender dropped")))
        })
    }
}
fn rig(
    cfg: SchedulerConfig,
    store: Option<Arc<dyn SchedulerStore>>,
) -> (
    Scheduler,
    SchedulerHandle,
    mpsc::UnboundedReceiver<Started>,
    Arc<Stats>,
) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let stats = Arc::new(Stats::default());
    let owner = Scheduler::new(
        context(),
        cfg,
        Arc::new(Executor {
            started: sender,
            stats: stats.clone(),
        }),
        store,
    )
    .unwrap();
    let handle = owner.handle();
    (owner, handle, receiver, stats)
}
async fn next(receiver: &mut mpsc::UnboundedReceiver<Started>) -> Started {
    tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap()
}
async fn wait(handle: &SchedulerHandle, ids: &[&str]) -> Vec<TaskSnapshot> {
    handle
        .wait(
            &ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
            WaitMode::All,
            Some(Duration::from_secs(2)),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn fresh_context_explicit_history_and_common_sync_background_engine() {
    let (owner, handle, mut starts, _) = rig(config(), None);
    let sync = handle.run(request("sync"));
    tokio::pin!(sync);
    let worker = async {
        let child = next(&mut starts).await;
        assert_eq!(child.invocation.context.run_id, "parent/sync");
        assert_eq!(child.invocation.depth, 1);
        assert_eq!(child.invocation.request.input, vec![text("work on sync")]);
        child
            .done
            .send(ChildOutcome::completed("sync done"))
            .unwrap();
    };
    let (result, ()) = tokio::join!(sync, worker);
    assert_eq!(result.unwrap().result, "sync done");
    let mut background = request("background");
    background.parent_history = Some(vec![text("explicit parent history")]);
    handle.submit(background).await.unwrap();
    let child = next(&mut starts).await;
    assert_eq!(
        child.invocation.request.input,
        vec![text("explicit parent history"), text("work on background")]
    );
    child
        .done
        .send(ChildOutcome::completed("background done"))
        .unwrap();
    wait(&handle, &["background"]).await;
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn atomic_dag_validation_and_dependency_forwarding() {
    let (owner, handle, mut starts, _) = rig(config(), None);
    let mut b = request("b");
    b.depends_on = vec!["a".into()];
    let mut a = request("a");
    a.depends_on = vec!["b".into()];
    assert!(handle.submit_dag(vec![a, b.clone()]).await.is_err());
    assert!(handle.list().is_empty());
    assert!(handle.submit(b.clone()).await.is_err());
    assert!(handle.list().is_empty());
    handle.submit_dag(vec![b, request("a")]).await.unwrap();
    let a = next(&mut starts).await;
    assert_eq!(a.invocation.task_id, "a");
    assert_eq!(
        handle.status("b", Detail::Full).unwrap().status,
        TaskStatus::Waiting
    );
    a.done
        .send(ChildOutcome::completed("upstream findings"))
        .unwrap();
    let b = next(&mut starts).await;
    assert_eq!(b.invocation.task_id, "b");
    assert_eq!(b.invocation.dependencies[0].result, "upstream findings");
    assert!(
        serde_json::to_string(&b.invocation.request.input)
            .unwrap()
            .contains("upstream findings")
    );
    b.done.send(ChildOutcome::completed("joined")).unwrap();
    wait(&handle, &["a", "b"]).await;
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn dependency_failure_policies_and_forwarding_opt_out() {
    let (owner, handle, mut starts, _) = rig(config(), None);
    let mut blocked = request("blocked");
    blocked.depends_on = vec!["a".into()];
    let mut always = request("always");
    always.depends_on = vec!["a".into()];
    always.dependency_policy = DependencyPolicy::AllTerminal;
    always.include_dependency_results = false;
    handle
        .submit_dag(vec![request("a"), blocked, always])
        .await
        .unwrap();
    next(&mut starts)
        .await
        .done
        .send(ChildOutcome::failed("upstream error"))
        .unwrap();
    let child = next(&mut starts).await;
    assert_eq!(child.invocation.task_id, "always");
    assert_eq!(child.invocation.request.input, vec![text("work on always")]);
    child.done.send(ChildOutcome::completed("cleanup")).unwrap();
    let tasks = wait(&handle, &["a", "blocked", "always"]).await;
    assert_eq!(
        tasks.iter().map(|t| t.status).collect::<Vec<_>>(),
        vec![
            TaskStatus::Failed,
            TaskStatus::Failed,
            TaskStatus::Completed
        ]
    );
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn shared_concurrency_budget_and_incremental_delivery() {
    let mut cfg = config();
    cfg.max_concurrency = 1;
    cfg.budget.tokens = Some(10);
    let (owner, handle, mut starts, stats) = rig(cfg, None);
    handle
        .submit_dag(vec![request("a"), request("b"), request("c")])
        .await
        .unwrap();
    let a = next(&mut starts).await;
    a.control
        .charge(BudgetUsage {
            tokens: 6,
            ..Default::default()
        })
        .await
        .unwrap();
    a.done
        .send(ChildOutcome {
            usage: BudgetUsage {
                tokens: 6,
                ..Default::default()
            },
            ..ChildOutcome::completed("a")
        })
        .unwrap();
    let b = next(&mut starts).await;
    assert_eq!(handle.collect_ids(&["a".into()]).await.unwrap().len(), 1);
    assert!(handle.collect_ids(&["a".into()]).await.unwrap().is_empty());
    b.done
        .send(ChildOutcome {
            usage: BudgetUsage {
                tokens: 4,
                ..Default::default()
            },
            ..ChildOutcome::completed("b")
        })
        .unwrap();
    let tasks = wait(&handle, &["a", "b", "c"]).await;
    assert_eq!(tasks[2].status, TaskStatus::Failed);
    assert_eq!(handle.snapshot().usage.tokens, 10);
    assert_eq!(stats.peak.load(Ordering::SeqCst), 1);
    assert_eq!(handle.collect().await.unwrap().len(), 2);
    assert!(handle.submit(request("after-budget")).await.is_err());
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn wait_any_ignores_delivered_and_activity_timeout_does_not_cancel() {
    let (owner, handle, mut starts, _) = rig(config(), None);
    handle
        .submit_dag(vec![request("a"), request("b")])
        .await
        .unwrap();
    let a = next(&mut starts).await;
    let b = next(&mut starts).await;
    a.done.send(ChildOutcome::completed("a")).unwrap();
    wait(&handle, &["a"]).await;
    handle.collect_ids(&["a".into()]).await.unwrap();
    b.control
        .activity(Activity {
            current_step: "thinking".into(),
            recent: vec!["event".into(); 40],
            ..Default::default()
        })
        .await
        .unwrap();
    let error = handle
        .wait(
            &["a".into(), "b".into()],
            WaitMode::Any,
            Some(Duration::from_millis(10)),
        )
        .await
        .unwrap_err();
    assert_eq!(error.info.category, ErrorCategory::DeadlineExceeded);
    assert_eq!(
        handle
            .status("b", Detail::Activity)
            .unwrap()
            .activity
            .unwrap()
            .recent
            .len(),
        30
    );
    assert!(
        handle
            .status("b", Detail::Summary)
            .unwrap()
            .activity
            .is_none()
    );
    assert_eq!(
        handle.status("b", Detail::Full).unwrap().status,
        TaskStatus::Running
    );
    b.done.send(ChildOutcome::completed("b")).unwrap();
    handle
        .wait(
            &["a".into(), "b".into()],
            WaitMode::Any,
            Some(Duration::MAX),
        )
        .await
        .unwrap();
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn steering_ids_acknowledgements_notifications_and_finish_gate() {
    let (owner, handle, mut starts, _) = rig(config(), None);
    handle.submit(request("a")).await.unwrap();
    let a = next(&mut starts).await;
    assert!(!a.control.messages_pending());
    let notifier = a.control.wait_for_messages();
    let steer = handle.steer("a", "m1", "new direction");
    let (notification, sent) = tokio::join!(notifier, steer);
    notification.unwrap();
    sent.unwrap();
    handle.steer("a", "m1", "new direction").await.unwrap();
    assert!(handle.steer("a", "m1", "different").await.is_err());
    assert_eq!(a.control.take_messages().await.unwrap()[0].id, "m1");
    handle.steer("a", "m2", "second").await.unwrap();
    let messages = a.control.take_messages().await.unwrap();
    assert_eq!(
        messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["m1", "m2"]
    );
    assert!(
        a.control
            .acknowledge_messages(&["unknown".into()])
            .await
            .is_err()
    );
    a.control
        .acknowledge_messages(&["m1".into()])
        .await
        .unwrap();
    assert_eq!(
        a.control.finish_or_take_messages().await.unwrap()[0].id,
        "m2"
    );
    a.control
        .acknowledge_messages(&["m2".into()])
        .await
        .unwrap();
    assert!(
        a.control
            .finish_or_take_messages()
            .await
            .unwrap()
            .is_empty()
    );
    assert!(handle.steer("a", "m3", "too late").await.is_err());
    a.done.send(ChildOutcome::completed("done")).unwrap();
    wait(&handle, &["a"]).await;
    handle.steer("a", "m1", "new direction").await.unwrap();
    assert_eq!(
        handle.status("a", Detail::Full).unwrap().messages_received,
        2
    );
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_joins_children_and_retained_handles_cannot_restart_owner() {
    let (owner, handle, mut starts, stats) = rig(config(), None);
    handle.submit(request("a")).await.unwrap();
    let a = next(&mut starts).await;
    assert_eq!(stats.active.load(Ordering::SeqCst), 1);
    owner.shutdown().await.unwrap();
    assert_eq!(stats.active.load(Ordering::SeqCst), 0);
    assert_eq!(stats.finished.load(Ordering::SeqCst), 1);
    assert_eq!(
        handle.status("a", Detail::Full).unwrap().status,
        TaskStatus::Cancelled
    );
    assert!(a.done.send(ChildOutcome::completed("late")).is_err());
    assert!(handle.submit(request("b")).await.is_err());
}

#[tokio::test]
async fn cancel_running_and_waiting_does_not_cancel_parent() {
    let (owner, handle, mut starts, stats) = rig(config(), None);
    let mut b = request("b");
    b.depends_on = vec!["a".into()];
    handle.submit_dag(vec![request("a"), b]).await.unwrap();
    let a = next(&mut starts).await;
    handle.cancel("a").await.unwrap();
    handle.cancel("b").await.unwrap();
    assert!(a.invocation.context.cancellation.is_cancelled());
    assert_eq!(
        wait(&handle, &["a", "b"]).await[0].status,
        TaskStatus::Cancelled
    );
    handle.submit(request("c")).await.unwrap();
    next(&mut starts)
        .await
        .done
        .send(ChildOutcome::completed("independent"))
        .unwrap();
    wait(&handle, &["c"]).await;
    owner.shutdown().await.unwrap();
    assert_eq!(stats.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn scoped_delegation_enforces_depth_and_yields_slot_without_deadlock() {
    let mut cfg = config();
    cfg.max_concurrency = 1;
    cfg.max_depth = 2;
    let (owner, handle, mut starts, stats) = rig(cfg, None);
    handle.submit(request("a")).await.unwrap();
    let a = next(&mut starts).await;
    let nested = a.control.delegation_handle();
    nested.submit(request("b")).await.unwrap();
    let ids = ["b".into()];
    let (result, ()) = tokio::join!(nested.wait(&ids, WaitMode::All, None), async {
        let b = next(&mut starts).await;
        assert_eq!(b.invocation.depth, 2);
        assert!(
            b.control
                .delegation_handle()
                .submit(request("c"))
                .await
                .is_err()
        );
        assert!(nested.status("a", Detail::Full).is_err());
        b.done.send(ChildOutcome::completed("done")).unwrap();
    });
    assert_eq!(result.unwrap()[0].status, TaskStatus::Completed);
    a.done
        .send(ChildOutcome::completed("joined child"))
        .unwrap();
    wait(&handle, &["a", "b"]).await;
    // Both futures remain owned; only the child with the execution slot is active.
    assert_eq!(stats.peak.load(Ordering::SeqCst), 2);
    owner.shutdown().await.unwrap();
}

struct Store {
    snapshots: Mutex<Vec<SchedulerCheckpoint>>,
    reader: Mutex<Option<SchedulerHandle>>,
    observed: Mutex<Vec<u64>>,
    blocked_revision: AtomicU64,
    failed_revision: AtomicU64,
    entered: Notify,
    release: Semaphore,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            snapshots: Mutex::new(vec![]),
            reader: Mutex::new(None),
            observed: Mutex::new(vec![]),
            blocked_revision: AtomicU64::new(0),
            failed_revision: AtomicU64::new(0),
            entered: Notify::new(),
            release: Semaphore::new(0),
        }
    }
}
impl SchedulerStore for Store {
    fn persist<'a>(
        &'a self,
        context: &'a Context,
        checkpoint: &'a SchedulerCheckpoint,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let reader = self.reader.lock().unwrap().clone();
            if let Some(reader) = reader {
                let value = reader.checkpoint(context).await?;
                self.observed
                    .lock()
                    .unwrap()
                    .push(value["revision"].as_u64().unwrap());
            }
            if self.blocked_revision.load(Ordering::SeqCst) == checkpoint.revision {
                self.entered.notify_one();
                self.release.acquire().await.unwrap().forget();
            }
            if self.failed_revision.load(Ordering::SeqCst) == checkpoint.revision {
                return Err(Error::new(ErrorCategory::Host, "injected persist failure"));
            }
            self.snapshots.lock().unwrap().push(checkpoint.clone());
            Ok(())
        })
    }
}

#[tokio::test]
async fn persistence_is_reentrant_committed_only_and_precedes_dispatch() {
    let store = Arc::new(Store::default());
    store.blocked_revision.store(2, Ordering::SeqCst);
    let (owner, handle, mut starts, _) = rig(config(), Some(store.clone()));
    *store.reader.lock().unwrap() = Some(handle.clone());
    handle.submit(request("a")).await.unwrap();
    store.entered.notified().await;
    assert!(starts.try_recv().is_err());
    assert_eq!(
        handle.status("a", Detail::Full).unwrap().status,
        TaskStatus::Pending
    );
    assert_eq!(handle.snapshot().revision, 1);
    assert_eq!(*store.observed.lock().unwrap(), vec![0, 1]);
    store.release.add_permits(1);
    let a = next(&mut starts).await;
    assert_eq!(
        store.snapshots.lock().unwrap()[1].records[0].task.status,
        TaskStatus::Running
    );
    a.done.send(ChildOutcome::completed("done")).unwrap();
    wait(&handle, &["a"]).await;
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_dispatch_persistence_never_executes_or_exposes_provisional_state() {
    let store = Arc::new(Store::default());
    store.failed_revision.store(2, Ordering::SeqCst);
    let (owner, handle, mut starts, _) = rig(config(), Some(store));
    handle.submit(request("a")).await.unwrap();
    let error = handle
        .wait(&["a".into()], WaitMode::All, Some(Duration::from_secs(1)))
        .await
        .unwrap_err();
    assert_eq!(error.info.category, ErrorCategory::Host);
    assert!(starts.try_recv().is_err());
    assert_eq!(handle.snapshot().revision, 1);
    assert_eq!(
        handle.status("a", Detail::Full).unwrap().status,
        TaskStatus::Pending
    );
    assert!(handle.submit(request("b")).await.is_err());
    assert!(owner.shutdown().await.is_err());
}

async fn queued_checkpoint() -> SchedulerCheckpoint {
    let (owner, handle, _, _) = rig(config(), None);
    handle.submit(request("a")).await.unwrap();
    let checkpoint = handle.snapshot();
    assert_eq!(checkpoint.records[0].dispatch, DispatchState::Never);
    owner.shutdown().await.unwrap();
    checkpoint
}

#[tokio::test]
async fn restore_never_dispatches_queued_work_without_explicit_resume() {
    let checkpoint = queued_checkpoint().await;
    let (owner, handle, mut starts, _) = rig(config(), None);
    handle
        .restore(&context(), serde_json::to_value(checkpoint).unwrap())
        .await
        .unwrap();
    tokio::task::yield_now().await;
    assert!(starts.try_recv().is_err());
    assert_eq!(
        handle.status("a", Detail::Full).unwrap().status,
        TaskStatus::Reconciling
    );
    handle.resume_queued("a").await.unwrap();
    next(&mut starts)
        .await
        .done
        .send(ChildOutcome::completed("resumed safely"))
        .unwrap();
    wait(&handle, &["a"]).await;
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn restored_running_effect_and_steering_journal_require_reconciliation() {
    let (old_owner, old_handle, mut old_starts, _) = rig(config(), None);
    old_handle.submit(request("a")).await.unwrap();
    let a = next(&mut old_starts).await;
    old_handle.steer("a", "one", "first").await.unwrap();
    a.control.take_messages().await.unwrap();
    old_handle.steer("a", "two", "second").await.unwrap();
    let checkpoint = old_handle.snapshot();
    old_owner.shutdown().await.unwrap();
    let (owner, handle, mut starts, _) = rig(config(), None);
    handle
        .restore(&context(), serde_json::to_value(checkpoint).unwrap())
        .await
        .unwrap();
    assert!(handle.resume_queued("a").await.is_err());
    let restored = handle.snapshot();
    assert_eq!(restored.records[0].in_flight_messages[0].id, "one");
    assert_eq!(restored.records[0].queued_messages[0].id, "two");
    handle
        .reconcile(
            "a",
            ChildOutcome::completed("operator confirmed destination effect"),
        )
        .await
        .unwrap();
    assert_eq!(handle.collect().await.unwrap().len(), 1);
    assert!(starts.try_recv().is_err());
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancel_wins_while_resume_persistence_is_blocked() {
    let checkpoint = queued_checkpoint().await;
    let store = Arc::new(Store::default());
    store
        .blocked_revision
        .store(checkpoint.revision + 1, Ordering::SeqCst);
    let (owner, handle, mut starts, _) = rig(config(), Some(store.clone()));
    handle
        .restore(&context(), serde_json::to_value(checkpoint).unwrap())
        .await
        .unwrap();
    let resume = handle.resume_queued("a");
    let concurrent = async {
        store.entered.notified().await;
        let cancel = handle.cancel("a");
        tokio::pin!(cancel);
        tokio::select! { result = &mut cancel => panic!("cancel unexpectedly completed: {result:?}"), _ = tokio::task::yield_now() => {} }
        store.release.add_permits(1);
        cancel.await.unwrap();
    };
    let (result, ()) = tokio::join!(resume, concurrent);
    result.unwrap();
    tokio::task::yield_now().await;
    assert_eq!(
        handle.status("a", Detail::Full).unwrap().status,
        TaskStatus::Cancelled
    );
    assert!(starts.try_recv().is_err());
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_reconciliation_rolls_back_and_preserves_delivery() {
    let checkpoint = queued_checkpoint().await;
    let store = Arc::new(Store::default());
    store
        .failed_revision
        .store(checkpoint.revision + 1, Ordering::SeqCst);
    let (owner, handle, _, _) = rig(config(), Some(store));
    handle
        .restore(&context(), serde_json::to_value(checkpoint).unwrap())
        .await
        .unwrap();
    assert!(
        handle
            .reconcile("a", ChildOutcome::completed("uncommitted"))
            .await
            .is_err()
    );
    let record = &handle.snapshot().records[0];
    assert_eq!(record.task.status, TaskStatus::Reconciling);
    assert!(record.task.result.is_empty());
    assert!(!record.result_delivered);
    assert!(owner.shutdown().await.is_err());
}

#[tokio::test]
async fn restore_rejects_inconsistent_or_weakened_baselines_atomically() {
    let mut checkpoint = queued_checkpoint().await;
    checkpoint.records[0]
        .security_baseline
        .input_guardrails
        .insert("required".into());
    let (owner, handle, _, _) = rig(config(), None);
    assert!(
        handle
            .restore(&context(), serde_json::to_value(&checkpoint).unwrap())
            .await
            .is_err()
    );
    assert!(handle.list().is_empty());
    checkpoint.records[0]
        .security_baseline
        .input_guardrails
        .clear();
    checkpoint.usage.tokens = 1;
    assert!(
        handle
            .restore(&context(), serde_json::to_value(&checkpoint).unwrap())
            .await
            .is_err()
    );
    assert!(handle.list().is_empty());
    owner.shutdown().await.unwrap();
}

#[test]
fn every_security_dimension_is_monotone() {
    use adk_core::{AccessMode, ApprovalPolicy};
    let mut saved = SecurityBaseline::default();
    saved.tools.allowed_tools = Some(["read".into()].into());
    saved.tools.denied_tools.insert("delete".into());
    saved.tools.approval = ApprovalPolicy::All;
    saved.tools.timeout = Some(Duration::from_secs(1));
    saved.input_guardrails.insert("input".into());
    saved.output_guardrails.insert("output".into());
    saved.untrusted_tool_outputs = true;
    saved.max_output_bytes = Some(32);
    assert!(saved.allows_resume_under(&saved));
    let weakenings: [fn(&mut SecurityBaseline); 10] = [
        |s| s.tools.access = AccessMode::FullAccess,
        |s| s.tools.allowed_tools = None,
        |s| s.tools.denied_tools.clear(),
        |s| {
            s.tools.allowed_mutating_tools.insert("write".into());
        },
        |s| s.tools.approval = ApprovalPolicy::RequiredByTool,
        |s| s.tools.timeout = None,
        |s| s.input_guardrails.clear(),
        |s| s.output_guardrails.clear(),
        |s| s.untrusted_tool_outputs = false,
        |s| s.max_output_bytes = None,
    ];
    for weaken in weakenings {
        let mut current = saved.clone();
        weaken(&mut current);
        assert!(!saved.allows_resume_under(&current));
    }
}

#[tokio::test]
async fn dropped_persistence_future_poison_is_latched_and_snapshot_fails_closed() {
    let store = Arc::new(Store::default());
    store.blocked_revision.store(1, Ordering::SeqCst);
    let (owner, handle, mut starts, _) = rig(config(), Some(store.clone()));
    {
        let submit = handle.submit(request("a"));
        tokio::pin!(submit);
        tokio::select! {
            result = &mut submit => panic!("unexpected submit: {result:?}"),
            _ = store.entered.notified() => {}
        }
        assert!(handle.list().is_empty());
        assert!(handle.checkpoint(&context()).await.is_ok());
    }
    assert!(handle.checkpoint(&context()).await.is_err());
    assert!(handle.submit(request("b")).await.is_err());
    assert!(starts.try_recv().is_err());
    assert!(owner.shutdown().await.is_err());
}

struct CasStore(AtomicU64);
impl SchedulerStore for CasStore {
    fn persist<'a>(
        &'a self,
        _: &'a Context,
        checkpoint: &'a SchedulerCheckpoint,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.0
                .compare_exchange(
                    checkpoint.revision - 1,
                    checkpoint.revision,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .map(|_| ())
                .map_err(|_| Error::new(ErrorCategory::Host, "stale scheduler revision"))
        })
    }
}

#[tokio::test]
async fn fenced_ledger_rejects_stale_parent_queued_snapshot() {
    let store = Arc::new(CasStore(AtomicU64::new(0)));
    let (old_owner, old_handle, mut old_starts, _) = rig(config(), Some(store.clone()));
    old_handle.submit(request("a")).await.unwrap();
    let stale = old_handle.snapshot();
    next(&mut old_starts)
        .await
        .done
        .send(ChildOutcome::completed("external effect finished"))
        .unwrap();
    wait(&old_handle, &["a"]).await;
    old_owner.shutdown().await.unwrap();
    let (owner, handle, mut starts, _) = rig(config(), Some(store));
    handle
        .restore(&context(), serde_json::to_value(stale).unwrap())
        .await
        .unwrap();
    assert!(handle.resume_queued("a").await.is_err());
    assert!(starts.try_recv().is_err());
    assert_eq!(
        handle.status("a", Detail::Full).unwrap().status,
        TaskStatus::Reconciling
    );
    assert!(owner.shutdown().await.is_err());
}

#[tokio::test]
async fn last_shared_turn_is_usable_but_next_turn_is_denied() {
    let mut cfg = config();
    cfg.budget.turns = Some(1);
    let (owner, handle, mut starts, _) = rig(cfg, None);
    handle.submit(request("a")).await.unwrap();
    let a = next(&mut starts).await;
    a.control
        .charge(BudgetUsage {
            turns: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    tokio::task::yield_now().await;
    assert!(!a.invocation.context.cancellation.is_cancelled());
    assert_eq!(a.control.usage().turns, 1);
    assert!(
        a.control
            .charge(BudgetUsage {
                turns: 1,
                ..Default::default()
            })
            .await
            .is_err()
    );
    a.done
        .send(ChildOutcome::failed("budget exhausted"))
        .unwrap();
    wait(&handle, &["a"]).await;
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn accepted_but_unprocessed_steering_cannot_disappear_as_success() {
    let (owner, handle, mut starts, _) = rig(config(), None);
    handle.submit(request("a")).await.unwrap();
    let a = next(&mut starts).await;
    handle
        .steer("a", "message", "must be processed")
        .await
        .unwrap();
    a.done
        .send(ChildOutcome::completed("executor ignored steering"))
        .unwrap();
    let _ = handle
        .wait(
            &["a".into()],
            WaitMode::All,
            Some(Duration::from_millis(10)),
        )
        .await;
    assert_eq!(
        handle.status("a", Detail::Full).unwrap().status,
        TaskStatus::Reconciling
    );
    assert_eq!(
        handle.snapshot().records[0].queued_messages[0].id,
        "message"
    );
    handle
        .reconcile("a", ChildOutcome::failed("operator rejected result"))
        .await
        .unwrap();
    owner.shutdown().await.unwrap();
}

struct PanicExecutor;
impl ChildExecutor for PanicExecutor {
    fn execute<'a>(
        &'a self,
        _: ChildInvocation,
        _: ChildControl,
    ) -> BoxFuture<'a, Result<ChildOutcome, Error>> {
        Box::pin(async { panic!("injected executor panic") })
    }
}
#[tokio::test]
async fn child_panic_has_terminal_evidence_and_does_not_kill_scheduler() {
    let owner = Scheduler::new(context(), config(), Arc::new(PanicExecutor), None).unwrap();
    let handle = owner.handle();
    handle.submit(request("a")).await.unwrap();
    let task = wait(&handle, &["a"]).await.remove(0);
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(task.error.as_deref(), Some("child executor panicked"));
    handle.submit(request("b")).await.unwrap();
    assert_eq!(wait(&handle, &["b"]).await[0].status, TaskStatus::Failed);
    owner.shutdown().await.unwrap();
}

struct Model(Arc<AtomicUsize>);
impl adk_core::Model for Model {
    fn provider(&self) -> &str {
        "fixture"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: adk_core::ModelRequest,
    ) -> BoxFuture<'a, Result<adk_core::ModelResponse, Error>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(adk_core::ModelResponse {
                items: vec![RunItem::Message {
                    message: Message {
                        role: Role::Assistant,
                        content: vec![Content::Text {
                            text: "native child result".into(),
                        }],
                    },
                }],
                usage: Default::default(),
                end_turn: Some(true),
                response_id: None,
                metadata: Default::default(),
            })
        })
    }
}
struct Host;
impl adk_core::Host for Host {
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
fn native_executor(calls: Arc<AtomicUsize>) -> Arc<dyn ChildExecutor> {
    use adk_runtime::{AgentConfig, ModelBinding, Runner, RunnerChildExecutor, RunnerConfig};
    let runner = Runner::new(
        AgentConfig::new(
            "worker",
            ModelBinding::complete("fixture", Arc::new(Model(calls))),
        ),
        RunnerConfig::default(),
    )
    .unwrap();
    Arc::new(RunnerChildExecutor::new(
        [("worker".into(), runner)].into(),
        Arc::new(Host),
    ))
}

#[tokio::test]
async fn native_completed_checkpoint_resumes_without_replaying_provider() {
    let calls = Arc::new(AtomicUsize::new(0));
    let executor = native_executor(calls.clone());
    let store = Arc::new(Store::default());
    let old_owner =
        Scheduler::new(context(), config(), executor.clone(), Some(store.clone())).unwrap();
    let old_handle = old_owner.handle();
    old_handle.submit(request("a")).await.unwrap();
    assert_eq!(
        wait(&old_handle, &["a"]).await[0].status,
        TaskStatus::Completed
    );
    let checkpoint = store
        .snapshots
        .lock()
        .unwrap()
        .iter()
        .find(|snapshot| {
            snapshot.records[0].task.status == TaskStatus::Running
                && snapshot.records[0]
                    .durable_checkpoint
                    .as_ref()
                    .is_some_and(|c| c.execution_boundary() == "run_completed")
        })
        .unwrap()
        .clone();
    old_owner.shutdown().await.unwrap();
    let owner = Scheduler::new(
        context(),
        config(),
        executor,
        Some(Arc::new(Store::default())),
    )
    .unwrap();
    let handle = owner.handle();
    handle
        .restore(&context(), serde_json::to_value(checkpoint).unwrap())
        .await
        .unwrap();
    handle.resume_checkpoint("a").await.unwrap();
    let task = wait(&handle, &["a"]).await.remove(0);
    assert_eq!(task.status, TaskStatus::Completed);
    assert_eq!(task.result, "native child result");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn native_dispatched_effect_is_never_automatically_resumed() {
    let calls = Arc::new(AtomicUsize::new(0));
    let executor = native_executor(calls.clone());
    let store = Arc::new(Store::default());
    let old_owner =
        Scheduler::new(context(), config(), executor.clone(), Some(store.clone())).unwrap();
    let old_handle = old_owner.handle();
    old_handle.submit(request("a")).await.unwrap();
    wait(&old_handle, &["a"]).await;
    let checkpoint = store
        .snapshots
        .lock()
        .unwrap()
        .iter()
        .find(|snapshot| {
            snapshot.records[0]
                .durable_checkpoint
                .as_ref()
                .and_then(|c| c.effect.as_ref())
                .is_some_and(|effect| effect.state == adk_durable::EffectState::Dispatched)
        })
        .unwrap()
        .clone();
    old_owner.shutdown().await.unwrap();
    let owner = Scheduler::new(
        context(),
        config(),
        executor,
        Some(Arc::new(Store::default())),
    )
    .unwrap();
    let handle = owner.handle();
    handle
        .restore(&context(), serde_json::to_value(checkpoint).unwrap())
        .await
        .unwrap();
    assert!(handle.resume_checkpoint("a").await.is_err());
    assert!(handle.resume_queued("a").await.is_err());
    assert_eq!(
        handle.status("a", Detail::Full).unwrap().status,
        TaskStatus::Reconciling
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn stricter_security_resume_checkpoint_roundtrips_again() {
    let checkpoint = queued_checkpoint().await;
    let mut cfg = config();
    cfg.security.tools.timeout = Some(Duration::from_secs(1));
    let (owner, handle, _, _) = rig(cfg.clone(), None);
    handle
        .restore(&context(), serde_json::to_value(checkpoint).unwrap())
        .await
        .unwrap();
    handle.resume_queued("a").await.unwrap();
    let resumed = handle.snapshot();
    assert_eq!(
        resumed.records[0].submission.policy.tools,
        resumed.records[0].security_baseline.tools
    );
    owner.shutdown().await.unwrap();
    let (owner, handle, _, _) = rig(cfg, None);
    handle
        .restore(&context(), serde_json::to_value(resumed).unwrap())
        .await
        .unwrap();
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn delegated_non_tool_security_survives_root_restore() {
    let mut cfg = config();
    cfg.max_concurrency = 1;
    let (owner, handle, mut starts, _) = rig(cfg.clone(), None);
    let mut submission = request("a");
    submission.security = Some(SecurityBaseline {
        max_output_bytes: Some(32),
        ..Default::default()
    });
    handle.submit(submission).await.unwrap();
    let a = next(&mut starts).await;
    a.control
        .delegation_handle()
        .submit(request("b"))
        .await
        .unwrap();
    let snapshot = handle.snapshot();
    assert_eq!(
        snapshot.records[1].security_baseline.max_output_bytes,
        Some(32)
    );
    owner.shutdown().await.unwrap();
    let (owner, handle, _, _) = rig(cfg, None);
    handle
        .restore(&context(), serde_json::to_value(snapshot).unwrap())
        .await
        .unwrap();
    assert_eq!(
        handle.snapshot().records[1]
            .security_baseline
            .max_output_bytes,
        Some(32)
    );
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn owner_drop_aborts_child_futures_and_closes_retained_handles() {
    let (owner, handle, mut starts, stats) = rig(config(), None);
    handle.submit(request("a")).await.unwrap();
    let child = next(&mut starts).await;
    drop(owner);
    tokio::time::timeout(Duration::from_secs(2), async {
        while stats.active.load(Ordering::SeqCst) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(handle.submit(request("b")).await.is_err());
    assert!(
        child
            .done
            .send(ChildOutcome::completed("too late"))
            .is_err()
    );
}

#[tokio::test]
async fn dropping_suspended_nested_wait_cancels_owned_subtree() {
    let mut cfg = config();
    cfg.max_concurrency = 1;
    let (owner, handle, mut starts, stats) = rig(cfg, None);
    handle.submit(request("a")).await.unwrap();
    let a = next(&mut starts).await;
    let nested = a.control.delegation_handle();
    nested.submit(request("b")).await.unwrap();
    let ids = ["b".into()];
    let mut waiting = Box::pin(nested.wait(&ids, WaitMode::All, None));
    let b = tokio::select! {
        result = &mut waiting => panic!("wait completed too early: {result:?}"),
        child = next(&mut starts) => child,
    };
    drop(waiting);
    let terminal = wait(&handle, &["a", "b"]).await;
    assert!(
        terminal
            .iter()
            .all(|task| task.status == TaskStatus::Cancelled)
    );
    drop((a, b));
    owner.shutdown().await.unwrap();
    assert_eq!(stats.active.load(Ordering::SeqCst), 0);
}

struct ToolModel(Arc<AtomicUsize>);
impl adk_core::Model for ToolModel {
    fn provider(&self) -> &str {
        "fixture"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: adk_core::ModelRequest,
    ) -> BoxFuture<'a, Result<adk_core::ModelResponse, Error>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            let items = if request
                .input
                .iter()
                .any(|item| matches!(item, RunItem::ToolResult { .. }))
            {
                vec![RunItem::Message {
                    message: Message {
                        role: Role::Assistant,
                        content: vec![Content::Text {
                            text: "after tool".into(),
                        }],
                    },
                }]
            } else {
                vec![RunItem::ToolCall {
                    call: adk_core::ToolCall {
                        id: "read-1".into(),
                        name: "read".into(),
                        arguments: serde_json::json!({}),
                    },
                }]
            };
            Ok(adk_core::ModelResponse {
                items,
                usage: Default::default(),
                end_turn: None,
                response_id: None,
                metadata: Default::default(),
            })
        })
    }
}
struct ReadTool(adk_core::ToolDefinition, Arc<AtomicUsize>);
impl adk_core::Tool for ReadTool {
    fn definition(&self) -> &adk_core::ToolDefinition {
        &self.0
    }
    fn execute<'a>(
        &'a self,
        _: &'a adk_core::ToolContext,
        _: adk_core::ToolCall,
    ) -> BoxFuture<'a, Result<adk_core::ToolOutput, Error>> {
        Box::pin(async move {
            self.1.fetch_add(1, Ordering::SeqCst);
            Ok(adk_core::ToolOutput {
                content: vec![Content::Text {
                    text: "read result".into(),
                }],
                is_error: false,
                should_pause: false,
            })
        })
    }
}

#[tokio::test]
async fn native_model_completed_and_tool_prepared_resume_only_remaining_work() {
    use adk_runtime::{AgentConfig, ModelBinding, Runner, RunnerChildExecutor, RunnerConfig};
    let model_calls = Arc::new(AtomicUsize::new(0));
    let tool_calls = Arc::new(AtomicUsize::new(0));
    let mut agent = AgentConfig::new(
        "worker",
        ModelBinding::complete("fixture", Arc::new(ToolModel(model_calls.clone()))),
    );
    agent.tools.push(Arc::new(ReadTool(
        adk_core::ToolDefinition {
            name: "read".into(),
            description: "read fixture".into(),
            input_schema: schemars::schema_for!(serde_json::Value),
            read_only: true,
            requires_approval: false,
        },
        tool_calls.clone(),
    )));
    let executor = Arc::new(RunnerChildExecutor::new(
        [(
            "worker".into(),
            Runner::new(agent, RunnerConfig::default()).unwrap(),
        )]
        .into(),
        Arc::new(Host),
    ));
    let store = Arc::new(Store::default());
    let owner = Scheduler::new(context(), config(), executor.clone(), Some(store.clone())).unwrap();
    owner.handle().submit(request("a")).await.unwrap();
    assert_eq!(
        wait(&owner.handle(), &["a"]).await[0].status,
        TaskStatus::Completed
    );
    owner.shutdown().await.unwrap();
    let snapshots = store.snapshots.lock().unwrap().clone();
    for boundary in ["model_completed", "tool_prepared"] {
        let checkpoint = snapshots
            .iter()
            .find(|snapshot| {
                snapshot.records[0]
                    .durable_checkpoint
                    .as_ref()
                    .is_some_and(|c| c.execution_boundary() == boundary)
            })
            .unwrap();
        let models_before = model_calls.load(Ordering::SeqCst);
        let tools_before = tool_calls.load(Ordering::SeqCst);
        let owner = Scheduler::new(
            context(),
            config(),
            executor.clone(),
            Some(Arc::new(Store::default())),
        )
        .unwrap();
        let handle = owner.handle();
        handle
            .restore(&context(), serde_json::to_value(checkpoint).unwrap())
            .await
            .unwrap();
        handle.resume_checkpoint("a").await.unwrap();
        let terminal = wait(&handle, &["a"]).await;
        assert_eq!(
            terminal[0].status,
            TaskStatus::Completed,
            "{boundary}: {terminal:?}"
        );
        assert_eq!(terminal[0].result, "after tool");
        assert_eq!(
            model_calls.load(Ordering::SeqCst) - models_before,
            1,
            "{boundary}"
        );
        assert_eq!(
            tool_calls.load(Ordering::SeqCst) - tools_before,
            1,
            "{boundary}"
        );
        owner.shutdown().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn scoped_wait_timeout_reacquires_slot_without_cancelling_parent() {
    let mut cfg = config();
    cfg.max_concurrency = 1;
    let (owner, handle, mut starts, _) = rig(cfg, None);
    handle.submit(request("a")).await.unwrap();
    let a = next(&mut starts).await;
    let scope = a.control.delegation_handle();
    scope.submit(request("b")).await.unwrap();
    let ids = ["b".into()];
    let (result, ()) = tokio::join!(
        scope.wait(&ids, WaitMode::All, Some(Duration::from_millis(1))),
        async {
            let b = next(&mut starts).await;
            tokio::time::sleep(Duration::from_millis(10)).await;
            b.done.send(ChildOutcome::completed("done")).unwrap();
        }
    );
    assert_eq!(
        result.unwrap_err().info.category,
        ErrorCategory::DeadlineExceeded
    );
    assert_eq!(
        handle.status("a", Detail::Summary).unwrap().status,
        TaskStatus::Running
    );
    scope.submit(request("c")).await.unwrap();
    tokio::task::yield_now().await;
    assert!(
        starts.try_recv().is_err(),
        "parent must hold its execution slot again"
    );
    a.done
        .send(ChildOutcome::completed("parent continues after timeout"))
        .unwrap();
    next(&mut starts)
        .await
        .done
        .send(ChildOutcome::completed("last descendant"))
        .unwrap();
    wait(&handle, &["a", "b", "c"]).await;
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn recorded_timing_and_steering_survive_checkpoints() {
    let (owner, handle, mut starts, _) = rig(config(), None);
    let before = chrono::Utc::now();
    handle.submit(request("timed")).await.unwrap();
    let child = next(&mut starts).await;
    let running = handle.status("timed", Detail::Full).unwrap();
    assert!(running.started_at.unwrap() >= before);
    assert!(running.started_at.unwrap() <= chrono::Utc::now());
    assert!(running.duration.is_none());
    assert!(running.elapsed().is_some());
    handle
        .steer("timed", "one", "  real\nmessage  ")
        .await
        .unwrap();
    handle
        .steer("timed", "two", &"é".repeat(170))
        .await
        .unwrap();
    handle
        .steer("timed", "one", "  real\nmessage  ")
        .await
        .unwrap();
    let pending = handle.status("timed", Detail::Full).unwrap();
    assert_eq!(pending.messages_received, 2);
    assert_eq!(
        pending.last_parent_message,
        format!("{}...", "é".repeat(157))
    );
    let messages = child.control.take_messages().await.unwrap();
    child
        .control
        .acknowledge_messages(&messages.iter().map(|m| m.id.clone()).collect::<Vec<_>>())
        .await
        .unwrap();
    child.done.send(ChildOutcome::completed("done")).unwrap();
    let completed = wait(&handle, &["timed"]).await.remove(0);
    let duration = completed.duration.unwrap();
    assert!(duration > Duration::ZERO);
    assert!(
        duration
            <= chrono::Utc::now()
                .signed_duration_since(before)
                .to_std()
                .unwrap()
    );
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert_eq!(
        handle.status("timed", Detail::Full).unwrap().elapsed(),
        Some(duration)
    );
    let encoded = serde_json::to_value(handle.snapshot()).unwrap();
    let decoded: SchedulerCheckpoint = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(decoded.records[0].task.duration, Some(duration));
    assert_eq!(decoded.records[0].task.started_at, running.started_at);
    let mut old = encoded;
    let task = old["records"][0]["task"].as_object_mut().unwrap();
    for field in ["started_at", "duration", "last_parent_message"] {
        task.remove(field);
    }
    let activity = task["activity"].as_object_mut().unwrap();
    for field in ["current_tool", "current_tool_input", "recent_activity"] {
        activity.remove(field);
    }
    let decoded: SchedulerCheckpoint = serde_json::from_value(old).unwrap();
    assert!(decoded.records[0].task.started_at.is_none());
    assert!(decoded.records[0].task.duration.is_none());
    assert!(decoded.records[0].task.last_parent_message.is_empty());
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancellation_and_failed_dependencies_record_terminal_metadata() {
    let (owner, handle, mut starts, _) = rig(config(), None);
    let mut dependent = request("dependent");
    dependent.depends_on = vec!["cancelled".into()];
    handle
        .submit_dag(vec![request("cancelled"), dependent])
        .await
        .unwrap();
    let _child = next(&mut starts).await;
    handle.cancel("cancelled").await.unwrap();
    let tasks = wait(&handle, &["cancelled", "dependent"]).await;
    assert_eq!(tasks[0].error.as_deref(), Some("cancellation requested"));
    assert_eq!(
        tasks[1].error.as_deref(),
        Some(
            "agent \"worker\" failed before start: dependency task(s) did not complete successfully: cancelled (cancelled)"
        )
    );
    assert!(tasks.iter().all(|t| t.duration.is_some()));
    handle.cancel("cancelled").await.unwrap();
    assert_eq!(
        handle.status("cancelled", Detail::Full).unwrap().duration,
        tasks[0].duration
    );
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn structured_activity_records_observed_tool_lifecycle() {
    let (owner, handle, mut starts, _) = rig(config(), None);
    handle.submit(request("activity")).await.unwrap();
    let child = next(&mut starts).await;
    let mut activity = Activity::default();
    activity.record_tool_start("Write", "actual.rs");
    child.control.activity(activity.clone()).await.unwrap();
    let snapshot = handle
        .status("activity", Detail::Activity)
        .unwrap()
        .activity
        .unwrap();
    assert_eq!(snapshot.current_tool, "Write");
    assert_eq!(snapshot.current_tool_input, "actual.rs");
    assert_eq!(snapshot.current_step, "implementing");
    assert!(snapshot.recent_activity.is_empty());
    let before = chrono::Utc::now();
    let start = std::time::Instant::now();
    tokio::time::sleep(Duration::from_millis(2)).await;
    let elapsed = start.elapsed();
    activity.record_tool_end("Write", "actual.rs", false, elapsed);
    assert_eq!(
        activity.recent_activity[0].duration_ms,
        elapsed.as_millis() as u64
    );
    assert!(activity.recent_activity[0].timestamp >= before);
    assert!(activity.recent_activity[0].timestamp <= chrono::Utc::now());
    assert!(activity.current_tool.is_empty());
    assert!(activity.files_written.contains("actual.rs"));
    for _ in 0..35 {
        let start = std::time::Instant::now();
        activity.record_tool_start("read_file", "failed.rs");
        activity.record_tool_end("read_file", "failed.rs", true, start.elapsed());
    }
    child.control.activity(activity).await.unwrap();
    let activity = handle
        .status("activity", Detail::Full)
        .unwrap()
        .activity
        .unwrap();
    assert_eq!(activity.recent_activity.len(), 30);
    assert!(activity.files_read.is_empty());
    assert_eq!(activity.last_tool, "read_file");
    assert!(activity.recent_activity.iter().all(|entry| entry.is_error));
    child.done.send(ChildOutcome::completed("done")).unwrap();
    wait(&handle, &["activity"]).await;
    owner.shutdown().await.unwrap();
}
