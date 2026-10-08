use super::*;
use adk_core::{ToolCall, ToolContext, ToolOutput};
use adk_project_state::{
    FilesystemOptions, FilesystemResolutionHost, PrimeOptions, ProjectStore, Store, StoreOptions,
};
use std::sync::Mutex;
use tokio::sync::Notify;

struct State {
    tools: Option<Vec<Arc<dyn Tool>>>,
    active: usize,
}
struct Shared {
    state: Mutex<State>,
    drained: Notify,
}
pub(super) struct Owner(Arc<Shared>);
impl Owner {
    pub(super) fn new(store: Arc<dyn Store>, actor: &str) -> Self {
        Self(Arc::new(Shared {
            state: Mutex::new(State {
                tools: Some(adk_project_state::tools::tools(store, actor)),
                active: 0,
            }),
            drained: Notify::new(),
        }))
    }
    pub(super) fn tools(&self, features: &ProjectStateFeatures) -> Vec<Arc<dyn Tool>> {
        self.0
            .state
            .lock()
            .unwrap()
            .tools
            .as_ref()
            .unwrap()
            .iter()
            .enumerate()
            .filter(|(_, tool)| {
                let name = &tool.definition().name;
                (features.task_tools && name.starts_with("task_"))
                    || (features.memory_tools && name.starts_with("memory_"))
                    || (features.prime_tool && name == "prime_context")
            })
            .map(|(index, tool)| {
                Arc::new(Handle {
                    definition: tool.definition().clone(),
                    index,
                    shared: self.0.clone(),
                }) as Arc<dyn Tool>
            })
            .collect()
    }
    pub(super) fn begin_close(&self) {
        self.0.state.lock().unwrap().tools.take();
    }
    pub(super) async fn close(&self) {
        self.begin_close();
        loop {
            let notified = self.0.drained.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.0.state.lock().unwrap().active == 0 {
                return;
            }
            notified.await;
        }
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.begin_close();
    }
}
struct Lease(Arc<Shared>);
impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap();
        state.active -= 1;
        if state.active == 0 {
            self.0.drained.notify_waiters();
        }
    }
}
struct Handle {
    definition: ToolDefinition,
    index: usize,
    shared: Arc<Shared>,
}
fn closed() -> Error {
    Error::new(ErrorCategory::Cancelled, "project state is closed")
}
impl Tool for Handle {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            context.operation.check_active()?;
            let (tool, lease) = {
                let mut state = self.shared.state.lock().unwrap();
                let tool = state.tools.as_ref().ok_or_else(closed)?[self.index].clone();
                state.active += 1;
                (tool, Lease(self.shared.clone()))
            };
            let owned_context = ToolContext {
                operation: context.operation.clone(),
                work_dir: context.work_dir.clone(),
                policy: context.policy.clone(),
                idempotency_key: context.idempotency_key.clone(),
            };
            let runtime = tokio::runtime::Handle::current();
            let result = tokio::task::spawn_blocking(move || {
                let _lease = lease;
                let tool = tool;
                owned_context.operation.check_active()?;
                runtime.block_on(tool.execute(&owned_context, call))
            })
            .await
            .map_err(|_| Error::new(ErrorCategory::Host, "project-state operation failed"))?;
            context.operation.check_active()?;
            if self.shared.state.lock().unwrap().tools.is_none() {
                return Err(closed());
            }
            result
        })
    }
}

pub(super) async fn open(
    injected: Option<Arc<dyn Store>>,
    host: Option<FilesystemResolutionHost>,
    config: &Config,
) -> Result<Arc<dyn Store>, Error> {
    if let Some(store) = injected {
        return Ok(store);
    }
    let host = host.ok_or_else(|| {
        invalid("project state requires an injected store or explicit filesystem host paths")
    })?;
    let options = FilesystemOptions {
        state_dir: config.project_state.state_dir.clone(),
        store: StoreOptions {
            project_id: config.project_state.project_id.clone(),
            work_dir: config.work_dir.clone(),
            actor: actor(config).into(),
            run_id: config.project_state.run_id.clone(),
        },
        ..Default::default()
    }
    .resolve(&host)
    .map_err(|_| invalid("project-state path resolution failed"))?;
    tokio::task::spawn_blocking(move || ProjectStore::filesystem(options))
        .await
        .map_err(|_| Error::new(ErrorCategory::Host, "project-state initialization failed"))?
        .map(|store| Arc::new(store) as Arc<dyn Store>)
        .map_err(|_| Error::new(ErrorCategory::Host, "project-state initialization failed"))
}
pub(super) fn actor(config: &Config) -> &str {
    [&config.project_state.actor, &config.agent_name]
        .into_iter()
        .map(|s| s.as_str())
        .find(|s| !s.trim().is_empty())
        .unwrap_or("agent")
}
pub(super) async fn prime(
    store: Arc<dyn Store>,
    config: &Config,
    working_state: &mut String,
) -> Result<(), ()> {
    let options = PrimeOptions {
        actor: actor(config).into(),
        active_task_id: config.project_state.active_task_id.clone(),
        ready_limit: 8,
        memory_limit: 8,
    };
    let prime = tokio::task::spawn_blocking(move || store.prime_context(options))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
    if !prime.trim().is_empty() {
        *working_state = [working_state.as_str(), &prime]
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn priming_preserves_existing_text_and_uses_active_task_without_claiming() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            ProjectStore::filesystem(FilesystemOptions {
                state_dir: dir.path().join("state"),
                store: StoreOptions {
                    project_id: "test".into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap(),
        );
        let task = store
            .create_task(adk_project_state::CreateTaskInput {
                title: "prime marker".into(),
                ..Default::default()
            })
            .unwrap();
        let config = Config {
            project_state: ProjectStateConfig {
                active_task_id: task.id.clone(),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut working = " existing text ".to_owned();
        prime(store.clone(), &config, &mut working).await.unwrap();
        assert!(working.starts_with(" existing text \n\n"));
        assert!(working.contains("prime marker"));
        assert_eq!(store.get_task(&task.id).unwrap().status, "open");
    }
    struct BlockWrite {
        started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl adk_project_state::StateHooks for BlockWrite {
        fn before_write(&self, _: &adk_project_state::Event) -> adk_project_state::Result<()> {
            if let Some(sender) = self.started.lock().unwrap().take() {
                let _ = sender.send(());
                self.release.lock().unwrap().recv().unwrap();
            }
            Ok(())
        }
    }
    #[tokio::test]
    async fn cancelled_caller_and_close_waiter_do_not_abandon_blocking_writes() {
        let dir = tempfile::tempdir().unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, receiver) = std::sync::mpsc::channel();
        let store = Arc::new(
            ProjectStore::filesystem(FilesystemOptions {
                state_dir: dir.path().join("state"),
                store: StoreOptions {
                    project_id: "test".into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap()
            .with_hooks(Arc::new(BlockWrite {
                started: Mutex::new(Some(started)),
                release: Mutex::new(receiver),
            })),
        );
        let owner = Owner::new(store.clone(), "actor");
        let tool = owner
            .tools(&ProjectStateFeatures {
                task_tools: true,
                ..Default::default()
            })
            .into_iter()
            .find(|t| t.definition().name == "task_create")
            .unwrap();
        let work = dir.path().to_owned();
        let call = tokio::spawn(async move {
            let context = ToolContext {
                operation: Context {
                    run_id: "test".into(),
                    cancellation: Arc::new(CancellationToken::new()),
                    deadline: None,
                },
                work_dir: work,
                policy: ToolPolicy::default(),
                idempotency_key: None,
            };
            tool.execute(
                &context,
                ToolCall {
                    raw_arguments: None,
                    id: "test".into(),
                    name: "task_create".into(),
                    arguments: serde_json::json!({"title":"single write"}),
                },
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(5), ready)
            .await
            .unwrap()
            .unwrap();
        call.abort();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), owner.close())
                .await
                .is_err()
        );
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), owner.close())
            .await
            .unwrap();
        assert_eq!(store.list_tasks().unwrap().len(), 1);
    }
    #[tokio::test]
    async fn priming_matches_pinned_runtime_builder_working_state() {
        let oracle: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../fixtures/project-state/runtime-observations.json"
        ))
        .unwrap();
        for case in oracle["cases"].as_array().unwrap() {
            let dir = tempfile::tempdir().unwrap();
            let config = Config {
                work_dir: dir.path().into(),
                project_state: ProjectStateConfig {
                    project_id: "offline".into(),
                    state_dir: dir.path().join("state"),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut working = " existing marker ".to_owned();
            if case["bits"].as_u64().unwrap() & 1 != 0 {
                let store = open(
                    None,
                    Some(FilesystemResolutionHost {
                        cwd: dir.path().into(),
                        home: None,
                    }),
                    &config,
                )
                .await
                .unwrap();
                prime(store, &config, &mut working).await.unwrap();
            }
            assert_eq!(
                working.replace(dir.path().to_str().unwrap(), "/fixture"),
                case["working_state"].as_str().unwrap()
            );
        }
    }
}
