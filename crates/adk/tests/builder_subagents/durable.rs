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

#[tokio::test]
async fn named_builder_restart_uses_historical_native_binding_without_tool_replay() {
    for empty_catalog in [false, true] {
        let old_model = Script::new(vec![response(vec![call("roleProbe")])]);
        let old_probe = Probe::new("roleProbe", true);
        let journal = Arc::new(Journal::default());
        let owner = builder(automatic_config(), &old_model)
            .extra_tools([old_probe.clone() as Arc<dyn Tool>])
            .subagent_host(Arc::new(TestHost::default()))
            .subagent_store(journal.clone())
            .subagent_generation_id("old")
            .build_session(&context())
            .await
            .unwrap();
        let handle = owner.handle();
        let scheduler = &handle.subagents().unwrap().scheduler;
        let id = scheduler
            .submit(Submission::new("reviewer", "inspect"))
            .await
            .unwrap();
        // This task has not dispatched yet; changing the catalog must not rebind it.
        let new_model = Script::new(vec![response(vec![message("new child")])]);
        let mut new_config = automatic_config();
        if empty_catalog {
            new_config.roles.clear();
        } else {
            new_config.roles[0].name = "researcher".into();
        }
        let owner = builder(new_config.clone(), &new_model)
            .subagent_host(Arc::new(TestHost::default()))
            .subagent_generation_id("new")
            .owned_session(owner)
            .build_session(&context())
            .await
            .unwrap();
        journal.stop_after_tool.store(true, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(5), journal.reached.notified())
            .await
            .unwrap();
        let saved = journal.entries.lock().unwrap().last().unwrap().clone();
        assert_eq!(saved.active_generation_id.as_deref(), Some("new"));
        assert_eq!(saved.records[0].generation_id.as_deref(), Some("old"));
        assert_eq!(
            saved
                .generations
                .iter()
                .find(|generation| generation.id == "new")
                .unwrap()
                .config
                .agents
                .is_empty(),
            empty_catalog
        );
        assert_eq!(old_probe.calls.load(Ordering::SeqCst), 1);
        drop(owner);
        tokio::task::yield_now().await;
        journal.stop_after_tool.store(false, Ordering::SeqCst);

        // The host reconstructs the historical assembly and owns its tool resources.
        let historical_model = Script::new(vec![response(vec![message("historical resumed")])]);
        let historical_probe = Probe::new("roleProbe", true);
        let mut resources = builder(automatic_config(), &historical_model)
            .extra_tools([historical_probe.clone() as Arc<dyn Tool>])
            .subagent_host(Arc::new(TestHost::default()))
            .build(&context())
            .await
            .unwrap();
        let historical = SchedulerGeneration {
            id: "old".into(),
            config: saved.generations[0].config.clone(),
            executor: Arc::new(adk::runtime::RunnerChildExecutor::new(
                [(
                    "reviewer".into(),
                    adk::runtime::Runner::new(
                        resources.specialists()["reviewer"].as_ref().clone(),
                        RunnerConfig {
                            output: adk::runtime::output::OutputPolicy {
                                untrusted: false,
                                ..Default::default()
                            },
                            local_compaction: adk::runtime::compaction::LocalCompactionPolicy {
                                enabled: false,
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                    )
                    .unwrap(),
                )]
                .into(),
                Arc::new(TestHost::default()),
            )),
        };
        let mut owner = builder(new_config, &new_model)
            .subagent_host(Arc::new(TestHost::default()))
            .subagent_store(journal)
            .subagent_generation_id("new")
            .subagent_historical_generations(vec![historical])
            .build_session(&context())
            .await
            .unwrap();
        let handle = owner.handle();
        let scheduler = &handle.subagents().unwrap().scheduler;
        scheduler
            .restore(&context(), serde_json::to_value(saved).unwrap())
            .await
            .unwrap();
        assert!(historical_model.requests.lock().unwrap().is_empty());
        assert!(new_model.requests.lock().unwrap().is_empty());
        assert_eq!(scheduler.agent_names().count(), usize::from(!empty_catalog));
        assert_eq!(
            scheduler.status(&id, Detail::Full).unwrap().status,
            TaskStatus::Reconciling
        );
        scheduler.resume_checkpoint(&id).await.unwrap();
        let result = scheduler
            .wait(&[id], WaitMode::All, Some(Duration::from_secs(5)))
            .await
            .unwrap();
        assert_eq!(result[0].status, TaskStatus::Completed, "{:?}", result[0]);
        assert_eq!(result[0].result, "historical resumed");
        assert_eq!(historical_probe.calls.load(Ordering::SeqCst), 0);
        assert_eq!(historical_model.requests.lock().unwrap().len(), 1);
        assert!(new_model.requests.lock().unwrap().is_empty());
        if empty_catalog {
            assert!(
                scheduler
                    .submit(Submission::new("reviewer", "new work"))
                    .await
                    .is_err()
            );
        } else {
            let id = scheduler
                .submit(Submission::new("researcher", "new work"))
                .await
                .unwrap();
            let result = scheduler
                .wait(&[id], WaitMode::All, Some(Duration::from_secs(5)))
                .await
                .unwrap();
            assert_eq!(result[0].result, "new child");
        }
        owner.close().await.unwrap();
        resources.close().await.unwrap();
    }
}

#[tokio::test]
async fn named_builder_settings_are_not_silently_ignored() {
    let model = Script::new(vec![]);
    for missing_host in [false, true] {
        let mut b = builder(automatic_config(), &model).subagent_generation_id("");
        if !missing_host {
            b = b.subagent_host(Arc::new(TestHost::default()));
        }
        assert_eq!(
            b.build_session(&context())
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
        .subagent_generation_id("same")
        .build_session(&context())
        .await
        .unwrap();
    let handle = owner.handle();
    assert!(
        builder(automatic_config(), &model)
            .subagent_host(Arc::new(TestHost::default()))
            .subagent_generation_id("same")
            .owned_session(owner)
            .build_session(&context())
            .await
            .is_err()
    );
    assert!(handle.is_closed());
}

#[tokio::test]
async fn empty_named_automatic_catalog_owns_scheduler_without_advertising_tools() {
    let model = Script::new(vec![]);
    for named in [false, true] {
        let mut config = automatic_config();
        config.roles.clear();
        let mut b = builder(config, &model).subagent_host(Arc::new(TestHost::default()));
        if named {
            b = b.subagent_generation_id("empty");
        }
        let mut bundle = b.build(&context()).await.unwrap();
        assert_eq!(bundle.session().subagents().is_some(), named);
        assert!(bundle.agent().tools.is_empty());
        assert!(bundle.specialists().is_empty());
        bundle.close().await.unwrap();
    }
    for missing_host in [false, true] {
        let mut config = automatic_config();
        config.roles.clear();
        config.features.as_mut().unwrap().subagents.task = missing_host;
        let mut b = builder(config, &model).subagent_generation_id("empty");
        if !missing_host {
            b = b.subagent_host(Arc::new(TestHost::default()));
        }
        assert_eq!(
            b.build_session(&context())
                .await
                .err()
                .unwrap()
                .info
                .category,
            ErrorCategory::InvalidInput
        );
    }
}
