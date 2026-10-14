use super::*;
use std::time::Duration;

const LIFECYCLE_TIMEOUT: Duration = Duration::from_secs(5);

// Gate the model BEFORE it emits a tool call. Blocking inside an unowned probe
// would not prove that the original ToolBundle survives the first turn.
#[derive(Default)]
struct BeforeToolModel {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
    dropped: Arc<AtomicUsize>,
    requests: Mutex<Vec<(String, ModelRequest)>>,
}
impl Model for BeforeToolModel {
    fn provider(&self) -> &str {
        "before-tool"
    }
    fn complete<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            let first = {
                let mut requests = self.requests.lock().unwrap();
                requests.push((context.run_id.clone(), request));
                requests.len() == 1
            };
            if first {
                let _active = ActiveCall(self.dropped.clone());
                self.started.notify_one();
                self.release.notified().await;
                context.check_active()?;
                Ok(response(vec![call("roleProbe")]))
            } else {
                Ok(response(vec![message("original child completed")]))
            }
        })
    }
}
impl StreamingModel for BeforeToolModel {
    fn stream<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            let response = self.complete(context, request).await?;
            Ok(Box::new(Events(Some(response))) as Box<dyn ModelStream>)
        })
    }
}

struct HostSessionFixture {
    parent: Arc<Script>,
    child: Arc<BeforeToolModel>,
    probe: Arc<Probe>,
}
impl HostSessionFixture {
    fn new() -> Self {
        Self {
            parent: Script::new(vec![]),
            child: Arc::new(BeforeToolModel::default()),
            probe: Probe::new("roleProbe", true),
        }
    }
    fn builder(&self, probe: Arc<Probe>) -> Builder {
        let mut c = config();
        c.roles[0].model_override = "worker/child".into();
        c.features.as_mut().unwrap().handoffs = false;
        c.features.as_mut().unwrap().subagents.task = true;
        c.features.as_mut().unwrap().tools = ["ExtraTools".into()].into();
        builder(c, &self.parent)
            .model("worker", Kind::Local, self.child.clone())
            .unwrap()
            // ExtraTools go through ToolBundle::specialist_view, not directly
            // into a hand-built RunnerChildExecutor.
            .extra_tools([probe as Arc<dyn Tool>])
            .subagent_host(Arc::new(TestHost::default()))
    }
    async fn owner(&self) -> SessionState {
        self.builder(self.probe.clone())
            .build_session(&named_context("host-session"))
            .await
            .unwrap()
    }
    async fn start(&self, bundle: &Bundle, operation: &Context) -> String {
        let result = spawn_tool(bundle)
            .execute(&tool_context(operation.clone()), background_call("start"))
            .await
            .unwrap();
        assert!(!result.is_error, "{result:?}");
        tokio::time::timeout(LIFECYCLE_TIMEOUT, self.child.started.notified())
            .await
            .expect("child must reach the pre-tool gate");
        assert_eq!(self.probe.calls.load(Ordering::SeqCst), 0);
        assert_eq!(self.child.dropped.load(Ordering::SeqCst), 0);
        let tasks = bundle.session().subagents().unwrap().scheduler.list();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, TaskStatus::Running);
        tasks[0].id.clone()
    }
    async fn finish(&self, handle: &SessionHandle, id: &str) {
        self.child.release.notify_one();
        let scheduler = &handle.subagents().unwrap().scheduler;
        let tasks = scheduler
            .wait(&[id.into()], WaitMode::All, Some(LIFECYCLE_TIMEOUT))
            .await
            .unwrap();
        assert_eq!(tasks[0].status, TaskStatus::Completed, "{:?}", tasks[0]);
        assert_eq!(tasks[0].result, "original child completed");
        assert_eq!(self.probe.calls.load(Ordering::SeqCst), 1);
        {
            let requests = self.child.requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0].0, format!("host-session/{id}"));
            assert_eq!(requests[1].0, requests[0].0);
            assert!(requests[1].1.input.iter().any(|item| matches!(
                item,
                RunItem::ToolResult { call_id, output }
                    if call_id == "roleProbe" && !output.is_error
            )));
        }
        let collected = scheduler.collect().await.unwrap();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].id, id);
        assert!(scheduler.collect().await.unwrap().is_empty());
    }
}
fn named_context(name: &str) -> Context {
    Context {
        run_id: name.into(),
        ..context()
    }
}
fn tool_context(operation: Context) -> ToolContext {
    ToolContext {
        operation,
        work_dir: ".".into(),
        policy: ToolPolicy::default(),
        idempotency_key: None,
    }
}
fn background_call(id: &str) -> ToolCall {
    ToolCall {
        raw_arguments: None,
        id: id.into(),
        name: "subagent".into(),
        arguments: json!({"message":"work", "mode":"background"}),
    }
}
fn spawn_tool(bundle: &Bundle) -> Arc<dyn Tool> {
    bundle
        .agent()
        .tools
        .iter()
        .find(|tool| tool.definition().name == "subagent")
        .unwrap()
        .clone()
}

#[tokio::test]
async fn host_session_child_uses_original_owned_tool_after_first_turn_closes() {
    let fixture = HostSessionFixture::new();
    let mut owner = fixture.owner().await;
    let handle = owner.handle();
    let first_cancel = CancellationToken::new();
    let first_context = Context {
        cancellation: Arc::new(first_cancel.clone()),
        ..named_context("first-turn")
    };
    let first_probe = Probe::new("roleProbe", true);
    let mut first = fixture
        .builder(first_probe.clone())
        .session(handle.clone())
        .build(&first_context)
        .await
        .unwrap();
    let first_tool = first
        .agent()
        .tools
        .iter()
        .find(|tool| tool.definition().name == "roleProbe")
        .unwrap()
        .clone();
    let id = fixture.start(&first, &first_context).await;
    first.close().await.unwrap();
    drop(first);
    first_cancel.cancel();
    // Negative control: this is a genuinely owner-bound ToolBundle handle.
    assert_eq!(
        first_tool
            .execute(
                &tool_context(context()),
                ToolCall {
                    raw_arguments: None,
                    id: "closed-turn".into(),
                    name: "roleProbe".into(),
                    arguments: json!({}),
                }
            )
            .await
            .unwrap_err()
            .info
            .category,
        ErrorCategory::Cancelled
    );

    let second_probe = Probe::new("roleProbe", true);
    let mut second = fixture
        .builder(second_probe.clone())
        .session(handle.clone())
        .build(&named_context("second-turn"))
        .await
        .unwrap();
    assert!(Arc::ptr_eq(
        handle.subagents().unwrap(),
        second.session().subagents().unwrap()
    ));
    let tasks = second.session().subagents().unwrap().scheduler.list();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, id);
    assert_eq!(tasks[0].status, TaskStatus::Running);
    assert!(!handle.is_closed());
    fixture.finish(&second.session(), &id).await;
    assert_eq!(first_probe.calls.load(Ordering::SeqCst), 0);
    assert_eq!(second_probe.calls.load(Ordering::SeqCst), 0);
    second.close().await.unwrap();
    owner.close().await.unwrap();
}

#[tokio::test]
async fn host_session_close_joins_blocked_child_before_return_and_is_idempotent() {
    let fixture = HostSessionFixture::new();
    let mut owner = fixture.owner().await;
    let handle = owner.handle();
    let mut turn = fixture
        .builder(Probe::new("roleProbe", true))
        .session(handle.clone())
        .build(&named_context("turn"))
        .await
        .unwrap();
    fixture.start(&turn, &named_context("turn")).await;
    let escaped = spawn_tool(&turn);
    tokio::time::timeout(LIFECYCLE_TIMEOUT, owner.close())
        .await
        .unwrap()
        .unwrap();
    // No yield/poll after close: the active future must already have been joined.
    assert_eq!(fixture.child.dropped.load(Ordering::SeqCst), 1);
    assert!(handle.is_closed());
    assert!(
        handle
            .subagents()
            .unwrap()
            .scheduler
            .submit(Submission::new("reviewer", "after close"))
            .await
            .is_err()
    );
    assert!(
        escaped
            .execute(&tool_context(context()), background_call("after-close"))
            .await
            .unwrap_err()
            .info
            .message
            .contains("child scheduler closed")
    );
    owner.close().await.unwrap();
    assert_eq!(fixture.child.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.probe.calls.load(Ordering::SeqCst), 0);
    turn.close().await.unwrap();
}

#[tokio::test]
async fn dropping_host_session_aborts_blocked_child_despite_escaped_handles() {
    let fixture = HostSessionFixture::new();
    let owner = fixture.owner().await;
    let handle = owner.handle();
    let children = handle.subagents().unwrap().clone();
    let mut turn = fixture
        .builder(Probe::new("roleProbe", true))
        .session(handle.clone())
        .build(&named_context("turn"))
        .await
        .unwrap();
    fixture.start(&turn, &named_context("turn")).await;
    let escaped = spawn_tool(&turn);
    drop(owner);
    assert!(handle.is_closed());
    assert!(
        children
            .scheduler
            .submit(Submission::new("reviewer", "after drop"))
            .await
            .is_err()
    );
    assert!(
        escaped
            .execute(&tool_context(context()), background_call("after-drop"))
            .await
            .unwrap_err()
            .info
            .message
            .contains("child scheduler closed")
    );
    tokio::time::timeout(LIFECYCLE_TIMEOUT, async {
        while fixture.child.dropped.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.child.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.probe.calls.load(Ordering::SeqCst), 0);
    turn.close().await.unwrap();
}

#[tokio::test]
async fn failed_host_session_factory_releases_resources_after_runner_validation() {
    let owner = SessionState::new();
    let handle = owner.handle();
    let model = Script::new(vec![]);
    let host = Arc::new(TestHost::default());
    let probe = Probe::new("roleProbe", true);
    let mut c = config();
    c.features.as_mut().unwrap().subagents.task = true;
    c.features.as_mut().unwrap().tools = ["ExtraTools".into()].into();
    // Parent Runner::new validates this only after ToolBundle construction and
    // automatic child runner initialization, exercising the late failure path.
    c.output_schema = Some(json!({"type":27}).try_into().unwrap());
    let error = invalid(
        builder(c, &model)
            .owned_session(owner)
            .subagent_host(host.clone())
            .extra_tools([probe.clone() as Arc<dyn Tool>])
            .build_session(&named_context("host-session"))
            .await,
    );
    assert!(error.info.message.contains("invalid JSON schema"));
    assert!(handle.is_closed());
    assert_eq!(Arc::strong_count(&host), 1);
    assert_eq!(Arc::strong_count(&model), 1);
    assert_eq!(Arc::strong_count(&probe), 1);
    assert!(model.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_borrowed_turn_build_preserves_host_session_and_existing_child() {
    let fixture = HostSessionFixture::new();
    let mut owner = fixture.owner().await;
    let handle = owner.handle();
    let mut first = fixture
        .builder(Probe::new("roleProbe", true))
        .session(handle.clone())
        .build(&named_context("first-turn"))
        .await
        .unwrap();
    let id = fixture.start(&first, &named_context("first-turn")).await;
    first.close().await.unwrap();
    drop(first);
    let mut bad = config();
    bad.features.as_mut().unwrap().subagents.task = true;
    bad.output_schema = Some(json!({"type":27}).try_into().unwrap());
    let error = invalid(
        builder(bad, &fixture.parent)
            .session(handle.clone())
            .build(&named_context("failed-turn"))
            .await,
    );
    assert!(error.info.message.contains("invalid JSON schema"));
    assert!(!handle.is_closed());
    assert_eq!(fixture.child.dropped.load(Ordering::SeqCst), 0);
    let mut next = fixture
        .builder(Probe::new("roleProbe", true))
        .session(handle.clone())
        .build(&named_context("next-turn"))
        .await
        .unwrap();
    assert!(Arc::ptr_eq(
        handle.subagents().unwrap(),
        next.session().subagents().unwrap()
    ));
    fixture.finish(&next.session(), &id).await;
    next.close().await.unwrap();
    owner.close().await.unwrap();
}

#[tokio::test]
async fn build_session_rejects_borrowed_owner_without_closing_it() {
    let fixture = HostSessionFixture::new();
    let mut owner = fixture.owner().await;
    let handle = owner.handle();
    let error = invalid(
        fixture
            .builder(Probe::new("roleProbe", true))
            .session(handle.clone())
            .build_session(&named_context("borrower"))
            .await,
    );
    assert!(error.info.message.contains("session ownership"));
    assert!(!handle.is_closed());
    let mut turn = fixture
        .builder(Probe::new("roleProbe", true))
        .session(handle.clone())
        .build(&named_context("turn"))
        .await
        .unwrap();
    let id = fixture.start(&turn, &named_context("turn")).await;
    fixture.finish(&handle, &id).await;
    turn.close().await.unwrap();
    owner.close().await.unwrap();
}

struct WaitingModel {
    started: tokio::sync::Notify,
    dropped: Arc<AtomicUsize>,
}
struct ActiveCall(Arc<AtomicUsize>);
impl Drop for ActiveCall {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl Model for WaitingModel {
    fn provider(&self) -> &str {
        "waiting"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            let _active = ActiveCall(self.dropped.clone());
            self.started.notify_one();
            std::future::pending().await
        })
    }
}
impl StreamingModel for WaitingModel {
    fn stream<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            let response = self.complete(context, request).await?;
            Ok(Box::new(Events(Some(response))) as Box<dyn ModelStream>)
        })
    }
}

#[tokio::test]
async fn owned_automatic_children_stop_on_close_and_drop_with_escaped_handles() {
    for explicit_close in [false, true] {
        let child = Arc::new(WaitingModel {
            started: tokio::sync::Notify::new(),
            dropped: Arc::new(AtomicUsize::new(0)),
        });
        let parent = Script::new(vec![]);
        let mut c = config();
        c.roles[0].model_override = "worker/waiting".into();
        c.features.as_mut().unwrap().handoffs = false;
        c.features.as_mut().unwrap().subagents.task = true;
        let mut bundle = builder(c, &parent)
            .model("worker", Kind::Local, child.clone())
            .unwrap()
            .subagent_host(Arc::new(TestHost::default()))
            .build(&context())
            .await
            .unwrap();
        let handle = bundle.session();
        let scheduler = handle.subagents().unwrap().scheduler.clone();
        let tool = bundle
            .agent()
            .tools
            .iter()
            .find(|t| t.definition().name == "subagent")
            .unwrap()
            .clone();
        let context = ToolContext {
            operation: context(),
            work_dir: ".".into(),
            policy: ToolPolicy::default(),
            idempotency_key: None,
        };
        let call = ToolCall {
            raw_arguments: None,
            id: "background".into(),
            name: "subagent".into(),
            arguments: json!({"message":"work","mode":"background"}),
        };
        let result = tool.execute(&context, call.clone()).await.unwrap();
        assert!(!result.is_error);
        tokio::time::timeout(Duration::from_secs(5), child.started.notified())
            .await
            .unwrap();
        if explicit_close {
            bundle.close().await.unwrap();
        }
        drop(bundle);
        tokio::time::timeout(Duration::from_secs(5), async {
            while child.dropped.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(child.dropped.load(Ordering::SeqCst), 1);
        assert!(handle.is_closed());
        assert!(
            scheduler
                .submit(Submission::new("reviewer", "after close"))
                .await
                .is_err()
        );
        assert_eq!(
            tool.execute(&context, call)
                .await
                .unwrap_err()
                .info
                .category,
            ErrorCategory::Cancelled
        );
    }
}
