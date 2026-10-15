use super::*;
use adk::runtime::ChildCheckpointOwner;
use std::{sync::atomic::AtomicBool, time::Duration};

#[derive(Default)]
struct Journal {
    entries: Mutex<Vec<SchedulerCheckpoint>>,
    stop_after_tool: AtomicBool,
    reached: tokio::sync::Notify,
    reject: bool,
}
impl SchedulerStore for Journal {
    fn persist<'a>(
        &'a self,
        _: &'a Context,
        checkpoint: &'a SchedulerCheckpoint,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            if self.reject {
                return Err(Error::new(
                    ErrorCategory::Host,
                    "injected persistence failure",
                ));
            }
            {
                let mut entries = self.entries.lock().unwrap();
                let previous = entries.last().map_or(0, |state| state.revision);
                assert_eq!(checkpoint.revision, previous + 1);
                entries.push(checkpoint.clone());
            }
            if self.stop_after_tool.load(Ordering::SeqCst)
                && checkpoint.records.iter().any(|record| {
                    record
                        .durable_checkpoint
                        .as_ref()
                        .is_some_and(|child| child.execution_boundary() == "tool_completed")
                })
            {
                self.reached.notify_one();
                std::future::pending::<()>().await;
            }
            Ok(())
        })
    }
}

fn automatic_config() -> Config {
    let mut config = config();
    config.features = Some(Features {
        subagents: SubagentFeatures {
            task: true,
            ..Default::default()
        },
        tools: ["ExtraTools".into()].into(),
        ..Default::default()
    });
    config
}

#[tokio::test]
async fn automatic_durable_session_restores_after_tool_without_replaying_it() {
    let model = Script::new(vec![
        response(vec![call("roleProbe")]),
        response(vec![message("recovered child")]),
    ]);
    let probe = Probe::new("roleProbe", true);
    let journal = Arc::new(Journal::default());
    journal.stop_after_tool.store(true, Ordering::SeqCst);
    let make_builder = || {
        builder(automatic_config(), &model)
            .extra_tools([probe.clone() as Arc<dyn Tool>])
            .subagent_host(Arc::new(TestHost::default()))
            .subagent_store(journal.clone())
    };
    let owner = make_builder().build_session(&context()).await.unwrap();
    let handle = owner.handle();
    let scheduler = &handle.subagents().unwrap().scheduler;
    assert!(scheduler.is_durable());
    let id = scheduler
        .submit(Submission::new("reviewer", "inspect"))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), journal.reached.notified())
        .await
        .unwrap();
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    let saved = journal.entries.lock().unwrap().last().unwrap().clone();
    assert_eq!(
        saved.records[0]
            .durable_checkpoint
            .as_ref()
            .unwrap()
            .execution_boundary(),
        "tool_completed"
    );
    drop(owner);
    tokio::task::yield_now().await;
    journal.stop_after_tool.store(false, Ordering::SeqCst);
    let mut owner = make_builder().build_session(&context()).await.unwrap();
    let handle = owner.handle();
    let scheduler = &handle.subagents().unwrap().scheduler;
    scheduler
        .restore(&context(), serde_json::to_value(saved).unwrap())
        .await
        .unwrap();
    assert_eq!(
        scheduler.status(&id, Detail::Full).unwrap().status,
        TaskStatus::Reconciling
    );
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    scheduler.resume_checkpoint(&id).await.unwrap();
    let tasks = scheduler
        .wait(&[id], WaitMode::All, Some(Duration::from_secs(5)))
        .await
        .unwrap();
    assert_eq!(tasks[0].status, TaskStatus::Completed, "{:?}", tasks[0]);
    assert_eq!(tasks[0].result, "recovered child");
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(model.requests.lock().unwrap().len(), 2);
    owner.close().await.unwrap();
}

#[tokio::test]
async fn automatic_durable_admission_failure_never_dispatches_model() {
    let model = Script::new(vec![response(vec![message("must not run")])]);
    let mut owner = builder(automatic_config(), &model)
        .subagent_host(Arc::new(TestHost::default()))
        .subagent_store(Arc::new(Journal {
            reject: true,
            ..Default::default()
        }))
        .build_session(&context())
        .await
        .unwrap();
    let handle = owner.handle();
    let scheduler = &handle.subagents().unwrap().scheduler;
    assert!(
        scheduler
            .submit(Submission::new("reviewer", "inspect"))
            .await
            .is_err()
    );
    assert!(scheduler.list().is_empty());
    assert!(model.requests.lock().unwrap().is_empty());
    let error = owner.close().await.unwrap_err();
    assert_eq!(error.info.category, ErrorCategory::Host);
    assert!(error.info.message.contains("ambiguous persistence failure"));
    assert!(handle.is_closed());
}

#[tokio::test]
async fn explicit_child_store_is_never_silently_ignored_or_replaced() {
    let model = Script::new(vec![]);
    for mode in [0, 1, 2] {
        let mut config = automatic_config();
        if mode == 1 {
            config.features.as_mut().unwrap().subagents.task = false;
        }
        if mode == 2 {
            config.roles.clear();
        }
        let mut builder = builder(config, &model).subagent_store(Arc::new(Journal::default()));
        if mode != 0 {
            builder = builder.subagent_host(Arc::new(TestHost::default()));
        }
        assert_eq!(
            builder
                .build_session(&context())
                .await
                .err()
                .unwrap()
                .info
                .category,
            ErrorCategory::InvalidInput
        );
    }
    let owner = builder(automatic_config(), &model)
        .subagent_host(Arc::new(TestHost::default()))
        .build_session(&context())
        .await
        .unwrap();
    let borrowed = builder(automatic_config(), &model)
        .subagent_host(Arc::new(TestHost::default()))
        .subagent_store(Arc::new(Journal::default()))
        .session(owner.handle());
    assert_eq!(
        borrowed
            .build(&context())
            .await
            .err()
            .unwrap()
            .info
            .category,
        ErrorCategory::InvalidInput
    );
    assert!(!owner.handle().is_closed());
    let handle = owner.handle();
    let replacement = builder(automatic_config(), &model)
        .subagent_host(Arc::new(TestHost::default()))
        .subagent_store(Arc::new(Journal::default()))
        .owned_session(owner)
        .build_session(&context())
        .await;
    assert_eq!(
        replacement.err().unwrap().info.category,
        ErrorCategory::InvalidInput
    );
    assert!(handle.is_closed());
    assert!(model.requests.lock().unwrap().is_empty());
}
