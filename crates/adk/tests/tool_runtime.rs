#![cfg(all(feature = "tools", feature = "runtime"))]
use adk::{
    core::*,
    runtime::*,
    tool_runtime::ToolRunner,
    tools::{Config, Features, bundle::BundleBuilder},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct TestTool {
    definition: ToolDefinition,
    calls: Arc<AtomicUsize>,
}
impl Tool for TestTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(ToolOutput {
                content: vec![],
                is_error: false,
                should_pause: false,
            })
        })
    }
}
fn tool(name: &str, calls: &Arc<AtomicUsize>) -> Arc<dyn Tool> {
    let mut definition = adk::tools::capabilities()
        .iter()
        .find_map(|c| c.definition.clone())
        .unwrap();
    definition.name = name.into();
    definition.read_only = true;
    Arc::new(TestTool {
        definition,
        calls: calls.clone(),
    })
}
#[derive(Default)]
struct TestModel {
    requests: Mutex<Vec<Vec<String>>>,
}
impl Model for TestModel {
    fn provider(&self) -> &str {
        "test"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request.tools.iter().map(|t| t.name.clone()).collect());
            let items = if requests.len() == 1 {
                ["visible", "hidden", "stale"]
                    .into_iter()
                    .map(|name| RunItem::ToolCall {
                        call: ToolCall {
                            id: name.into(),
                            name: name.into(),
                            arguments: Default::default(),
                        },
                    })
                    .collect()
            } else {
                vec![]
            };
            Ok(ModelResponse {
                items,
                usage: Usage::default(),
                end_turn: Some(requests.len() > 1),
                response_id: None,
                metadata: Default::default(),
            })
        })
    }
}
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
        panic!("unexpected approval")
    }
}
fn context() -> Context {
    Context {
        run_id: "runtime-bundle".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn request() -> RunRequest {
    RunRequest {
        input: vec![],
        policy: RunPolicy::default(),
    }
}
#[tokio::test]
async fn model_and_dispatch_use_exactly_prepared_tools_not_agent_or_request_tools() {
    let calls = Arc::new(AtomicUsize::new(0));
    let config = Config {
        features: Features::Strict(["ExtraTools".into()].into()),
        allowed_names: Some(["visible".into(), "hidden".into()].into()),
        ..Default::default()
    };
    let bundle = BundleBuilder::new(config)
        .extra_tools([tool("visible", &calls), tool("hidden", &calls)])
        .build(ToolPolicy {
            denied_tools: ["hidden".into()].into(),
            ..Default::default()
        })
        .unwrap();
    let model = Arc::new(TestModel::default());
    let mut agent = AgentConfig::new("agent", ModelBinding::complete("test", model.clone()));
    agent.tools.push(tool("stale", &calls));
    let mut runner = ToolRunner::new(agent, RunnerConfig::default(), bundle).unwrap();
    let mut req = request();
    req.policy.tools.allowed_tools = Some(["stale".into()].into());
    runner
        .run(context(), req, Arc::new(TestHost))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|names| names == &["visible"]));
    drop(requests);
    runner.close().await.unwrap();
    let error = runner
        .run(context(), request(), Arc::new(TestHost))
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::Cancelled);
}

#[tokio::test]
async fn stream_cannot_outlive_bundle_authority() {
    let bundle = BundleBuilder::new(Config::default())
        .build(ToolPolicy::default())
        .unwrap();
    let model = Arc::new(TestModel::default());
    let agent = AgentConfig::new("agent", ModelBinding::complete("test", model));
    let runner = ToolRunner::new(agent, RunnerConfig::default(), bundle).unwrap();
    let stream = runner.stream(context(), request(), Arc::new(TestHost));
    drop(runner);
    let error = stream.finish().await.err().unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::Cancelled);
}
