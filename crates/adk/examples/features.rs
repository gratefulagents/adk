//! Deterministic offline assertions: cargo run -p adk --all-features --example features -- all
use adk::{core::*, runtime::*};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

const SCENARIOS: &[&str] = &[
    "agent_runtime",
    "model_abstraction",
    "providers",
    "tools",
    "tools_registry",
    "mcp",
    "sandbox",
    "chatloop",
    "handoffs_subagents",
    "guardrails",
    "structured_output",
    "streaming",
    "context_compaction",
    "settings_routing",
    "observability",
    "errors_retries",
    "costs",
    "policy",
    "memory",
    "tracestore",
];

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
            ..Usage::default()
        },
        end_turn: None,
        response_id: None,
        metadata: Default::default(),
    }
}
fn answer(text: &str) -> ModelResponse {
    response(vec![message(Role::Assistant, text)])
}
fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: format!("call-{name}"),
        name: name.into(),
        arguments,
    }
}
fn tool_response(name: &str, arguments: Value) -> ModelResponse {
    response(vec![RunItem::ToolCall {
        call: call(name, arguments),
    }])
}
fn context() -> Context {
    Context {
        run_id: "offline-feature".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn request() -> RunRequest {
    RunRequest {
        input: vec![message(Role::User, "Exercise this feature")],
        policy: RunPolicy {
            max_turns: 5.try_into().unwrap(),
            ..RunPolicy::default()
        },
    }
}
fn tool_context() -> ToolContext {
    ToolContext {
        operation: context(),
        work_dir: ".".into(),
        policy: ToolPolicy::default(),
        idempotency_key: None,
    }
}
fn definition(name: &str, approval: bool) -> ToolDefinition {
    ToolDefinition { name: name.into(), description: "Offline lookup".into(),
        input_schema: json!({"type":"object","properties":{"key":{"type":"string"}},"required":["key"],"additionalProperties":false}).try_into().unwrap(),
        read_only: true, requires_approval: approval }
}

struct Scripted {
    replies: Mutex<VecDeque<Result<ModelResponse, Error>>>,
    requests: Mutex<Vec<ModelRequest>>,
}
impl Scripted {
    fn new(replies: Vec<ModelResponse>) -> Arc<Self> {
        Self::results(replies.into_iter().map(Ok).collect())
    }
    fn results(replies: Vec<Result<ModelResponse, Error>>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into()),
            requests: Mutex::new(vec![]),
        })
    }
}
impl Model for Scripted {
    fn provider(&self) -> &str {
        "offline"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected model invocation")
        })
    }
}
struct Events(VecDeque<ModelEvent>);
impl ModelStream for Events {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async { Ok(self.0.pop_front()) })
    }
}
impl StreamingModel for Scripted {
    fn stream<'a>(
        &'a self,
        ctx: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            let response = self.complete(ctx, request).await?;
            Ok(Box::new(Events(VecDeque::from([
                ModelEvent::TextDelta {
                    delta: "hel".into(),
                },
                ModelEvent::TextDelta { delta: "lo".into() },
                ModelEvent::Complete { response },
            ]))) as Box<dyn ModelStream>)
        })
    }
}
#[derive(Default)]
struct RecordingHost(Mutex<Vec<RunEvent>>);
impl Host for RecordingHost {
    fn emit<'a>(&'a self, _: &'a Context, event: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(event);
            Ok(())
        })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { Ok(ApprovalDecision::Defer) })
    }
}
struct Lookup {
    definition: ToolDefinition,
    calls: AtomicUsize,
}
impl Lookup {
    fn new(approval: bool) -> Arc<Self> {
        Arc::new(Self {
            definition: definition("lookup", approval),
            calls: AtomicUsize::new(0),
        })
    }
}
impl Tool for Lookup {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            assert_eq!(call.arguments["key"], "answer");
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ToolOutput {
                content: vec![Content::Text { text: "42".into() }],
                is_error: false,
                should_pause: false,
            })
        })
    }
}
fn agent(model: Arc<dyn Model>) -> AgentConfig {
    AgentConfig::new("assistant", ModelBinding::complete("offline-model", model))
}
fn runner(agent: AgentConfig) -> Runner {
    Runner::new(agent, RunnerConfig::default()).unwrap()
}
async fn run(agent: AgentConfig) -> RunOutcome {
    runner(agent)
        .run(context(), request(), Arc::new(RecordingHost::default()))
        .await
        .unwrap()
}

async fn agent_runtime() {
    let model = Scripted::new(vec![
        tool_response("lookup", json!({"key":"answer"})),
        answer("42"),
    ]);
    let tool = Lookup::new(false);
    let mut a = agent(model.clone());
    a.instructions = "Use lookup before answering".into();
    a.tools.push(tool.clone());
    let result = run(a).await.result;
    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.final_output, Some(json!("42")));
    assert_eq!(result.responses.len(), 2);
    assert_eq!(result.usage.input_tokens, 20);
    assert_eq!(result.usage.output_tokens, 4);
    assert_eq!(result.last_agent.as_deref(), Some("assistant"));
    assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
    {
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests[0].instructions, "Use lookup before answering");
        assert!(requests[1].input.iter().any(
            |item| matches!(item, RunItem::ToolResult { call_id, .. } if call_id == "call-lookup")
        ));
    }

    let mut req = request();
    req.policy.max_turns = 1.try_into().unwrap();
    let mut a = agent(Scripted::new(vec![tool_response(
        "lookup",
        json!({"key":"answer"}),
    )]));
    a.tools.push(Lookup::new(false));
    let error = runner(a)
        .run(context(), req, Arc::new(RecordingHost::default()))
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::MaxTurns);
    assert_eq!(error.partial.unwrap().responses.len(), 1);
}

async fn model_abstraction() {
    let selected = Scripted::new(vec![answer("routed")]);
    let unused = Scripted::new(vec![]);
    let mut routes = adk::providers::routing::Routes::new("mock");
    routes.register("mock", selected.clone()).unwrap();
    routes.register("other", unused.clone()).unwrap();
    assert_eq!(routes.resolve("fast").unwrap().1, "fast");
    assert!(routes.resolve("missing/model").is_err());
    let result = run(AgentConfig::new(
        "router",
        ModelBinding::complete("mock/vendor/model", Arc::new(routes)),
    ))
    .await;
    assert_eq!(result.result.final_output, Some(json!("routed")));
    assert_eq!(selected.requests.lock().unwrap()[0].model, "vendor/model");
    assert!(unused.requests.lock().unwrap().is_empty());
}

async fn providers() {
    use adk::providers::{
        auth::AuthMode,
        factory::{Kind, RouteSpec},
        wire::{self, Protocol},
    };
    assert_eq!(Kind::OpenRouter.protocol(), Protocol::Chat);
    assert_eq!(Kind::Anthropic.protocol(), Protocol::Anthropic);
    assert!(
        RouteSpec::new(Kind::Anthropic, AuthMode::AnthropicOAuth)
            .scope()
            .is_ok()
    );
    assert!(
        RouteSpec::new(Kind::Anthropic, AuthMode::OpenAiOAuth)
            .scope()
            .is_err()
    );
    let request = ModelRequest {
        model: "vendor/model".into(),
        instructions: "Be concise".into(),
        input: vec![message(Role::User, "hello")],
        tools: vec![],
        output_schema: None,
        output_schema_name: "answer".into(),
        output_schema_strict: true,
        settings: Default::default(),
    };
    for protocol in [Protocol::Chat, Protocol::Responses, Protocol::Anthropic] {
        let wire = wire::request(&request, protocol, false).unwrap();
        assert_eq!(wire["model"], "vendor/model");
        assert!(wire.to_string().contains("hello"));
        assert!(wire.to_string().contains("Be concise"));
    }
    let chat = wire::response(&json!({"id":"chat-1","choices":[{"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":7,"completion_tokens":2}}), Protocol::Chat).unwrap();
    let anthropic = wire::response(&json!({"id":"ant-1","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","usage":{"input_tokens":7,"output_tokens":2}}), Protocol::Anthropic).unwrap();
    assert_eq!(chat.items, anthropic.items);
    assert_eq!(chat.usage.input_tokens, 7);
    assert_eq!(anthropic.usage.output_tokens, 2);
}

async fn tools() {
    let tool = Lookup::new(true);
    let model = Scripted::new(vec![
        tool_response("lookup", json!({"key":"answer"})),
        answer("approved"),
    ]);
    let mut a = agent(model);
    a.tools.push(tool.clone());
    let paused = run(a).await;
    assert_eq!(paused.result.status, RunStatus::Paused);
    assert_eq!(paused.result.pending_approvals.len(), 1);
    assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
    let resumed = paused
        .continuation
        .unwrap()
        .resume(Some(ApprovalDecision::Approve))
        .await
        .unwrap();
    assert_eq!(resumed.result.final_output, Some(json!("approved")));
    assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
}

async fn tools_registry() {
    use adk::tools::{Config, Features, bundle::BundleBuilder};
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("example.txt"), "registry evidence").unwrap();
    let config = Config {
        features: Features::Strict(["ReadFile".into()].into()),
        ..Default::default()
    };
    let mut bundle = BundleBuilder::new(config)
        .build(ToolPolicy::default())
        .unwrap();
    let prepared = bundle.prepared();
    let tool = prepared
        .tools
        .iter()
        .find(|t| t.definition().name == "read_file")
        .unwrap();
    let mut ctx = tool_context();
    ctx.work_dir = temp.path().into();
    ctx.policy = prepared.policy;
    let output = tool
        .execute(&ctx, call("read_file", json!({"path":"example.txt"})))
        .await
        .unwrap();
    assert!(!output.is_error);
    assert!(
        serde_json::to_string(&output)
            .unwrap()
            .contains("registry evidence")
    );
    bundle.close().await.unwrap();
    let error = tool
        .execute(&ctx, call("read_file", json!({"path":"example.txt"})))
        .await
        .unwrap_err();
    assert_eq!(error.info.category, ErrorCategory::Cancelled);
}

struct McpManager(AtomicUsize);
impl adk::mcp::tools::ToolManager for McpManager {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![definition("mcp__offline__lookup", false)]
    }
    fn call<'a>(
        &'a self,
        name: &'a str,
        args: Value,
    ) -> BoxFuture<'a, Result<Value, adk::mcp::Error>> {
        Box::pin(async move {
            assert_eq!(name, "mcp__offline__lookup");
            assert_eq!(args["key"], "answer");
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"content":[{"type":"text","text":"42"}],"isError":false}))
        })
    }
    fn list_resources<'a>(
        &'a self,
        _: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Value, adk::mcp::Error>> {
        Box::pin(async { Ok(json!([])) })
    }
    fn read_resource<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Value, adk::mcp::Error>> {
        Box::pin(async { Ok(json!({"contents":[]})) })
    }
    fn has_resources(&self) -> bool {
        false
    }
}
async fn mcp() {
    let manager = Arc::new(McpManager(AtomicUsize::new(0)));
    let tools = adk::mcp::tools::build_tools(manager.clone());
    assert_eq!(tools.len(), 1);
    let output = tools[0]
        .execute(
            &tool_context(),
            call("mcp__offline__lookup", json!({"key":"answer"})),
        )
        .await
        .unwrap();
    assert_eq!(output.content, vec![Content::Text { text: "42".into() }]);
    assert!(!output.is_error);
    let invalid = tools[0]
        .execute(&tool_context(), call("mcp__offline__lookup", json!([])))
        .await
        .unwrap();
    assert!(invalid.is_error);
    assert_eq!(manager.0.load(Ordering::SeqCst), 1);
}

async fn sandbox() {
    use adk::sandbox::{Backend, Completion, Config, Executor, Network, Request};
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config::new(temp.path());
    config.backend = Backend::Local;
    config.output_limit = 64;
    let executor = Executor::new(config).unwrap();
    let mut req = Request::new("/bin/sh");
    req.args = vec![
        "-c".into(),
        "printf '%s' \"${PATH}\"; i=0; while [ $i -lt 100 ]; do printf x; i=$((i+1)); done".into(),
    ];
    req.access = AccessMode::FullAccess;
    req.network = Network::Allow;
    req.timeout = Some(Duration::from_secs(5));
    let output = executor.run(&context(), req.clone()).await.unwrap();
    assert_eq!(output.completion, Completion::Exited);
    assert!(output.status.success());
    assert_eq!(output.stdout.len() + output.stderr.len(), 64);
    assert!(output.truncated);
    req.args = vec!["-c".into(), "printf '%s|%s|%s' \"$PATH\" \"${OPENAI_API_KEY-unset}\" \"${AWS_SECRET_ACCESS_KEY-unset}\"".into()];
    let output = executor.run(&context(), req).await.unwrap();
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "/usr/bin:/bin|unset|unset"
    );
}

struct Gate(AtomicUsize);
impl compat::GoApprovalGate for Gate {
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        request: &'a ApprovalRequest,
    ) -> BoxFuture<'a, Result<compat::GoApprovalDecision, Error>> {
        Box::pin(async move {
            assert_eq!(request.call.name, "lookup");
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(compat::GoApprovalDecision {
                approved: true,
                reason: "offline approval".into(),
            })
        })
    }
}
async fn chatloop() {
    let model = Scripted::new(vec![
        tool_response("lookup", json!({"key":"answer"})),
        answer("first"),
        answer("second"),
    ]);
    let tool = Lookup::new(true);
    let mut a = agent(model.clone());
    a.tools.push(tool.clone());
    let runner = runner(a);
    let gate = Gate(AtomicUsize::new(0));
    let first = compat::run_go_chat(
        &runner,
        context(),
        request(),
        Arc::new(RecordingHost::default()),
        &gate,
        None,
    )
    .await
    .unwrap();
    assert_eq!(first.result.final_output, Some(json!("first")));
    assert_eq!(gate.0.load(Ordering::SeqCst), 1);
    assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
    let mut next = request();
    next.input = first.result.history;
    next.input.push(message(Role::User, "continue"));
    let second = runner
        .run(context(), next.clone(), Arc::new(RecordingHost::default()))
        .await
        .unwrap();
    assert_eq!(second.result.final_output, Some(json!("second")));
    assert_eq!(model.requests.lock().unwrap()[2].input, next.input);
    assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
}

async fn handoffs_subagents() {
    let specialist = Scripted::new(vec![answer("specialist evidence")]);
    let mut a = agent(Scripted::new(vec![tool_response(
        "transfer",
        json!({"key":"answer"}),
    )]));
    a.handoffs.push(Handoff {
        definition: definition("transfer", false),
        target: Arc::new(AgentConfig::new(
            "specialist",
            ModelBinding::complete("offline", specialist.clone()),
        )),
    });
    let result = run(a).await.result;
    assert_eq!(result.final_output, Some(json!("specialist evidence")));
    assert_eq!(result.last_agent.as_deref(), Some("specialist"));
    assert_eq!(specialist.requests.lock().unwrap().len(), 1);

    use subagent::{Scheduler, SchedulerConfig, SecurityBaseline};
    let child = Scripted::new(vec![answer("child evidence")]);
    let host = Arc::new(RecordingHost::default());
    let executor = RunnerChildExecutor::new(
        [("child".into(), runner(agent(child.clone())))]
            .into_iter()
            .collect(),
        host,
    );
    let baseline = SecurityBaseline::default();
    let owner = Scheduler::new(
        context(),
        SchedulerConfig {
            agents: [("child".into(), baseline)].into_iter().collect(),
            ..Default::default()
        },
        Arc::new(executor),
        None,
    )
    .unwrap();
    let session = Arc::new(SubagentSession::new(owner.handle()));
    let parent = Scripted::new(vec![
        tool_response("specialist", json!({"message":"investigate"})),
        answer("synthesis"),
    ]);
    let mut a = agent(parent.clone());
    a.tools.push(Arc::new(AgentAsTool::new(
        "specialist",
        "Consult child",
        "child",
        session.clone(),
    )));
    let r = Runner::new(
        a,
        RunnerConfig {
            subagents: Some(session),
            ..Default::default()
        },
    )
    .unwrap();
    let result = r
        .run(context(), request(), Arc::new(RecordingHost::default()))
        .await
        .unwrap();
    assert_eq!(result.result.final_output, Some(json!("synthesis")));
    assert!(
        serde_json::to_string(&parent.requests.lock().unwrap()[1].input)
            .unwrap()
            .contains("child evidence")
    );
    assert_eq!(child.requests.lock().unwrap().len(), 1);
    owner.shutdown().await.unwrap();
}

async fn guardrails() {
    adk::security::check_secrets("public documentation").unwrap();
    let secret = ["ghp_", &"a".repeat(36)].concat();
    let error = adk::security::check_secrets(&secret).unwrap_err();
    assert_eq!(error.info.category, ErrorCategory::Guardrail);
    assert!(!error.info.message.contains(&secret));
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("fixture"), &secret).unwrap();
    let mut config = adk::sandbox::Config::new(temp.path());
    config.backend = adk::sandbox::Backend::Local;
    let executor = adk::sandbox::Executor::new(config).unwrap();
    let policy = adk::security::SecurityPolicy {
        tools: ToolPolicy {
            access: AccessMode::FullAccess,
            ..Default::default()
        },
        allow_network: true,
        ..Default::default()
    };
    let command =
        adk::security::CommandRequest::from_call(call("shell", json!({"command":"cat fixture"})))
            .unwrap();
    let ctx = context();
    let adk::security::Authorization::Approved(command) = policy
        .authorize(
            &ctx,
            &RecordingHost::default(),
            &definition("shell", false),
            command,
        )
        .await
        .unwrap()
    else {
        panic!("unapproved read-only command");
    };
    let error = adk::execution::run_authorized(&executor, &ctx, command)
        .await
        .unwrap_err();
    assert_eq!(error.info.category, ErrorCategory::Guardrail);
    assert!(!error.info.message.contains(&secret));
}
async fn structured_output() {
    for (raw, valid) in [(r#"{"answer":42}"#, true), (r#"{"answer":"wrong"}"#, false)] {
        let mut a = agent(Scripted::new(vec![answer(raw)]));
        a.output_schema = Some(json!({"type":"object","properties":{"answer":{"type":"integer"}},"required":["answer"],"additionalProperties":false}).try_into().unwrap());
        let hooks = Arc::new(Hooks::default());
        let r = Runner::new(
            a,
            RunnerConfig {
                hooks: Some(hooks.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let result = r
            .run(context(), request(), Arc::new(RecordingHost::default()))
            .await
            .unwrap();
        assert_eq!(
            result.result.final_output,
            Some(serde_json::from_str(raw).unwrap())
        );
        let violations = hooks
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|o| matches!(o, Observation::OutputValidationFailed { .. }))
            .count();
        assert_eq!(violations, usize::from(!valid));
    }
}
async fn streaming() {
    let a = AgentConfig::new(
        "stream",
        ModelBinding::streaming("offline", Scripted::new(vec![answer("hello")])),
    );
    let mut stream = runner(a).stream(context(), request(), Arc::new(RecordingHost::default()));
    let mut deltas = vec![];
    let mut finished = 0;
    while let Some(event) = stream.next().await {
        match event {
            RunEvent::Model {
                event: ModelEvent::TextDelta { delta },
            } => deltas.push(delta),
            RunEvent::Finished { .. } => finished += 1,
            _ => {}
        }
    }
    assert_eq!(deltas, ["hel", "lo"]);
    assert_eq!(finished, 1);
    assert_eq!(
        stream.finish().await.unwrap().result.final_output,
        Some(json!("hello"))
    );
}
async fn context_compaction() {
    use compaction::{LocalCompactionPolicy, compact_for_request, extract_summary};
    let mut history = vec![message(Role::User, "Preserve the original goal")];
    for i in 0..30 {
        history.push(message(
            Role::Assistant,
            &format!("step {i}: {}", "historical detail ".repeat(100)),
        ));
    }
    history.push(message(Role::User, "Keep this latest question"));
    let policy = LocalCompactionPolicy {
        trigger_tokens: 1000,
        target_tokens: 500,
        preserve_recent_items: 1,
        preserve_initial_user_messages: 1,
        ..Default::default()
    };
    let compacted = compact_for_request(&history, policy, 0);
    assert!(compacted.changed);
    assert!(compacted.after_tokens < compacted.before_tokens);
    assert!(!extract_summary(&compacted.history).is_empty());
    assert!(compacted.history.contains(&history[0]));
    assert_eq!(compacted.history.last(), history.last());
    let disabled = compact_for_request(
        &history,
        LocalCompactionPolicy {
            enabled: false,
            ..policy
        },
        0,
    );
    assert!(!disabled.changed);
    assert_eq!(disabled.history, history);
}
async fn settings_routing() {
    use adk::builder::{Builder, Config, Features, ModeSpec, ModelRouting, RoleRouting, RoleSpec};
    for role in [None, Some("writer")] {
        let model = Scripted::new(vec![answer("configured")]);
        let config = Config {
            model: "mock/base".into(),
            active_role: role.map(str::to_owned),
            roles: vec![RoleSpec {
                name: "writer".into(),
                instructions: "Write clearly".into(),
                ..Default::default()
            }],
            features: Some(Features {
                mode_model_routing: true,
                mode_instructions: true,
                ..Default::default()
            }),
            mode_snapshot: Some(ModeSpec {
                name: "plan".into(),
                tool_access: "read-only".into(),
                instructions: "Inspect first".into(),
                model_routing: Some(ModelRouting {
                    default_model: "mock/planner".into(),
                    reasoning_level: "high".into(),
                    role_overrides: [(
                        "writer".into(),
                        RoleRouting {
                            model: "mock/writer".into(),
                            text_verbosity: "low".into(),
                            ..Default::default()
                        },
                    )]
                    .into(),
                    settings: [("temperature".into(), json!(0.2))].into_iter().collect(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut bundle = Builder::new(config)
            .model("mock", adk::providers::factory::Kind::Local, model.clone())
            .unwrap()
            .build(&context())
            .await
            .unwrap();
        assert_eq!(bundle.policy().tools.access, AccessMode::ReadOnly);
        let result = bundle
            .run(
                context(),
                request().input,
                Arc::new(RecordingHost::default()),
            )
            .await
            .unwrap();
        assert_eq!(result.result.final_output, Some(json!("configured")));
        {
            let requests = model.requests.lock().unwrap();
            assert_eq!(
                requests[0].model,
                if role.is_some() { "writer" } else { "planner" }
            );
            assert_eq!(requests[0].settings["temperature"], json!(0.2));
            assert_eq!(requests[0].settings["reasoning_effort"], "high");
            assert!(requests[0].instructions.contains("Inspect first"));
            if role.is_some() {
                assert_eq!(requests[0].settings["text_verbosity"], "low");
                assert!(requests[0].instructions.contains("Write clearly"));
            }
        }
        bundle.close().await.unwrap();
        assert!(bundle.session().is_closed());
    }
}
#[derive(Default)]
struct Hooks(Mutex<Vec<Observation>>);
impl RunHooks for Hooks {
    fn observe<'a>(
        &'a self,
        _: &'a Context,
        observation: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(observation);
            Ok(())
        })
    }
}
async fn observability() {
    use adk::observability::{CapturePolicy, Observability, ObservedHost};
    let sink = Arc::new(EventLines::default());
    let hooks = Arc::new(
        Observability::new(
            "offline-feature",
            CapturePolicy::default(),
            vec![sink.clone()],
        )
        .unwrap(),
    );
    let host = Arc::new(RecordingHost::default());
    let mut a = agent(Scripted::new(vec![
        tool_response("lookup", json!({"key":"answer"})),
        answer("observed"),
    ]));
    a.tools.push(Lookup::new(false));
    let r = Runner::new(
        a,
        RunnerConfig {
            hooks: Some(hooks.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    r.run(
        context(),
        request(),
        Arc::new(ObservedHost {
            host: host.clone(),
            observations: hooks.clone(),
        }),
    )
    .await
    .unwrap();
    let snapshot = hooks.snapshot().await;
    assert_eq!(snapshot.model_attempts, 2);
    assert_eq!(snapshot.tool_calls, 1);
    assert_eq!(snapshot.tool_results, 1);
    assert_eq!(snapshot.usage.input_tokens, 20);
    hooks.shutdown().await.unwrap();
    let bytes = sink.0.lock().unwrap().clone();
    let mut decoder = adk::observability::LineDecoder::new(1024 * 1024).unwrap();
    let mut records = vec![];
    for chunk in bytes.chunks(7) {
        records.extend(decoder.push(chunk).into_iter().map(Result::unwrap));
    }
    assert!(decoder.finish().is_none());
    assert_eq!(records.len() as u64, snapshot.sequence);
    assert!(
        records
            .iter()
            .enumerate()
            .all(|(i, r)| r.sequence == i as u64 + 1)
    );
    assert!(records.iter().any(|r| r.kind == "tool_start"));
    assert_eq!(records.last().unwrap().kind, "done");
    assert!(!String::from_utf8(bytes).unwrap().contains("observed"));
    let events = host.0.lock().unwrap();
    assert!(matches!(events.first(), Some(RunEvent::Started { .. })));
    assert!(matches!(events.last(), Some(RunEvent::Finished { .. })));
}
#[derive(Default)]
struct EventLines(Mutex<Vec<u8>>);
impl adk::observability::EventSink for EventLines {
    fn emit<'a>(
        &'a self,
        record: &'a adk::observability::EventRecord,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().extend(record.to_json_line()?);
            Ok(())
        })
    }
}
async fn tracestore() {
    use adk::observability::{
        CapturePolicy, EventRecord, FilesystemTraceStore, Observability, TraceLimits,
    };
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("private");
    let store = Arc::new(
        FilesystemTraceStore::create(&root, "offline-feature", TraceLimits::default()).unwrap(),
    );
    let pipeline =
        Observability::new("offline-feature", CapturePolicy::default(), vec![store]).unwrap();
    pipeline
        .publish(&context(), "first", json!({"output":"private content"}))
        .await
        .unwrap();
    pipeline
        .publish(&context(), "second", json!({"input_tokens":12}))
        .await
        .unwrap();
    pipeline.shutdown().await.unwrap();
    assert_eq!(pipeline.health().await.events_written, 2);
    let path = root.join("traces/offline-feature/events.jsonl");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let bytes = std::fs::read_to_string(path).unwrap();
    assert!(!bytes.contains("private content"));
    let records: Vec<_> = bytes
        .lines()
        .map(|line| EventRecord::from_json_line(line.as_bytes()).unwrap())
        .collect();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].sequence, 1);
    assert_eq!(records[1].sequence, 2);
    assert_eq!(records[1].data["input_tokens"], 12);
    assert!(FilesystemTraceStore::create(&root, "../escape", TraceLimits::default()).is_err());
    assert!(
        FilesystemTraceStore::create(&root, "offline-feature", TraceLimits::default()).is_err()
    );
}
async fn errors_retries() {
    let model = Scripted::results(vec![
        Err(Error::new(ErrorCategory::Provider, "temporary")),
        Ok(answer("recovered")),
    ]);
    let mut config = RunnerConfig::default();
    config.retry.max_retries = 1;
    config.retry.initial_delay = Duration::ZERO;
    let result = Runner::new(agent(model.clone()), config)
        .unwrap()
        .run(context(), request(), Arc::new(RecordingHost::default()))
        .await
        .unwrap();
    assert_eq!(result.result.final_output, Some(json!("recovered")));
    assert_eq!(model.requests.lock().unwrap().len(), 2);
    assert_eq!(result.result.responses.len(), 1);
    let token = CancellationToken::new();
    token.cancel();
    let mut ctx = context();
    ctx.cancellation = Arc::new(token);
    let never = Scripted::new(vec![]);
    let error = runner(agent(never.clone()))
        .run(ctx, request(), Arc::new(RecordingHost::default()))
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::Cancelled);
    assert!(never.requests.lock().unwrap().is_empty());
}
struct Prices;
impl CostEstimator for Prices {
    fn cost(&self, _: &str, usage: &Usage) -> f64 {
        usage.input_tokens as f64 * 0.001 + usage.output_tokens as f64 * 0.002
    }
}
async fn costs() {
    let hooks = Arc::new(Hooks::default());
    let r = Runner::new(
        agent(Scripted::new(vec![answer("costed")])),
        RunnerConfig {
            cost_estimator: Some(Arc::new(Prices)),
            hooks: Some(hooks.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    r.run(context(), request(), Arc::new(RecordingHost::default()))
        .await
        .unwrap();
    let costs: Vec<_> = hooks
        .0
        .lock()
        .unwrap()
        .iter()
        .filter_map(|o| {
            if let Observation::Usage { cost, .. } = o {
                Some(*cost)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(costs.len(), 1);
    assert!((costs[0] - 0.014).abs() < 1e-12);
    let mut a = agent(Scripted::new(vec![tool_response(
        "lookup",
        json!({"key":"answer"}),
    )]));
    let tool = Lookup::new(false);
    a.tools.push(tool.clone());
    let r = Runner::new(
        a,
        RunnerConfig {
            cost_estimator: Some(Arc::new(Prices)),
            limits: Limits {
                max_cost: Some(0.001),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    let error = r
        .run(context(), request(), Arc::new(RecordingHost::default()))
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::Guardrail);
    assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
}
async fn policy() {
    let mut tool = definition("write", true);
    tool.read_only = false;
    let mut policy = ToolPolicy::default();
    assert_eq!(policy.decision(&tool), ToolDecision::Deny);
    policy.access = AccessMode::WorkspaceWrite;
    assert_eq!(policy.decision(&tool), ToolDecision::RequireApproval);
    policy.denied_tools.insert("write".into());
    assert_eq!(policy.decision(&tool), ToolDecision::Deny);
    policy.denied_tools.clear();
    policy.allowed_tools = Some(["write_other".into()].into());
    assert_eq!(policy.decision(&tool), ToolDecision::Deny);
    assert_eq!(
        adk::security::clamp_access(AccessMode::FullAccess, AccessMode::ReadOnly),
        AccessMode::ReadOnly
    );
}
/// An application-owned backend adapter: the tool depends only on Store.
#[derive(Default)]
struct AuditedMemory {
    inner: adk::project_state::memory::InMemoryStore,
    operations: Mutex<Vec<&'static str>>,
}
impl adk::project_state::memory::Store for AuditedMemory {
    fn store(
        &self,
        namespace: &str,
        content: &str,
        tags: &[String],
        source_run: &str,
        metadata: Value,
    ) -> adk::project_state::Result<adk::project_state::memory::Memory> {
        self.operations.lock().unwrap().push("store");
        self.inner
            .store(namespace, content, tags, source_run, metadata)
    }
    fn search(
        &self,
        namespace: &str,
        query: &str,
        tags: &[String],
        limit: i32,
    ) -> adk::project_state::Result<Vec<adk::project_state::memory::Memory>> {
        self.operations.lock().unwrap().push("search");
        self.inner.search(namespace, query, tags, limit)
    }
    fn list(
        &self,
        namespace: &str,
        tags: &[String],
        limit: i32,
    ) -> adk::project_state::Result<Vec<adk::project_state::memory::Memory>> {
        self.operations.lock().unwrap().push("list");
        self.inner.list(namespace, tags, limit)
    }
    fn delete(&self, namespace: &str, id: uuid::Uuid) -> adk::project_state::Result<()> {
        self.operations.lock().unwrap().push("delete");
        self.inner.delete(namespace, id)
    }
}

async fn memory() {
    use adk::project_state::memory::Store;
    let store = Arc::new(AuditedMemory::default());
    let memory = store
        .store(
            "team-a",
            "Prefer Rust examples",
            &["preference".into()],
            "run-1",
            json!({"source":"example"}),
        )
        .unwrap();
    store
        .store("team-b", "Prefer Rust examples", &[], "run-2", json!({}))
        .unwrap();
    let found = store
        .search("team-a", "Rust examples", &["preference".into()], 5)
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, memory.id);
    assert_eq!(found[0].similarity, 1.0);
    assert!(store.delete("team-b", memory.id).is_err());
    store.delete("team-a", memory.id).unwrap();
    assert!(store.list("team-a", &[], 5).unwrap().is_empty());
    assert_eq!(store.list("team-b", &[], 5).unwrap().len(), 1);
    let tool = adk::tools::memory::tool(
        store.clone(),
        "team-a",
        "tool-run",
        "https://example.invalid/repo",
    );
    let mut ctx = tool_context();
    ctx.policy.access = AccessMode::WorkspaceWrite;
    let output = tool.execute(&ctx, call("Memory", json!({"action":"store","content":"Use namespace tools","tags":["guide"],"namespace":"team-b"}))).await.unwrap();
    assert!(!output.is_error);
    let memories = store.list("team-a", &[], 5).unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].source_run, "tool-run");
    assert_eq!(memories[0].metadata["repo"], "https://example.invalid/repo");
    assert_eq!(store.list("team-b", &[], 5).unwrap().len(), 1);
    let output = tool
        .execute(
            &ctx,
            call(
                "Memory",
                json!({"action":"search","content":"namespace tools"}),
            ),
        )
        .await
        .unwrap();
    assert!(!output.is_error);
    assert!(
        serde_json::to_string(&output)
            .unwrap()
            .contains("Use namespace tools")
    );
    let output = tool
        .execute(
            &ctx,
            call("Memory", json!({"action":"delete","id":memories[0].id})),
        )
        .await
        .unwrap();
    assert!(!output.is_error);
    assert!(store.list("team-a", &[], 5).unwrap().is_empty());
    let operations = store.operations.lock().unwrap();
    assert_eq!(operations.iter().filter(|op| **op == "store").count(), 3);
    assert_eq!(operations.iter().filter(|op| **op == "search").count(), 2);
    assert_eq!(operations.iter().filter(|op| **op == "delete").count(), 3);
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--list"] {
        for name in SCENARIOS {
            println!("{name}");
        }
        return;
    }
    let selected: Vec<&str> = if args.is_empty() || args == ["all"] {
        SCENARIOS.to_vec()
    } else {
        args.iter().map(String::as_str).collect()
    };
    for name in &selected {
        if !SCENARIOS.contains(name) {
            eprintln!("Unknown scenario {name:?}; use --list");
            std::process::exit(2);
        }
    }
    for name in selected {
        match name {
            "agent_runtime" => agent_runtime().await,
            "model_abstraction" => model_abstraction().await,
            "providers" => providers().await,
            "tools" => tools().await,
            "tools_registry" => tools_registry().await,
            "mcp" => mcp().await,
            "sandbox" => sandbox().await,
            "chatloop" => chatloop().await,
            "handoffs_subagents" => handoffs_subagents().await,
            "guardrails" => guardrails().await,
            "structured_output" => structured_output().await,
            "streaming" => streaming().await,
            "context_compaction" => context_compaction().await,
            "settings_routing" => settings_routing().await,
            "observability" => observability().await,
            "errors_retries" => errors_retries().await,
            "costs" => costs().await,
            "policy" => policy().await,
            "memory" => memory().await,
            "tracestore" => tracestore().await,
            _ => unreachable!(),
        }
        println!("PASS {name}");
    }
}
