#![cfg(feature = "host")]
use adk::{builder, host, observability};
use adk::{
    core::*,
    runtime::{AgentConfig, CancellationToken, ModelBinding, Observation, RunHooks},
};
use host::*;
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

type MessagePage = Result<(Vec<UserMessage>, Cursor), Error>;

#[derive(Default)]
struct Fixture {
    events: Mutex<Vec<String>>,
    pages: Mutex<VecDeque<MessagePage>>,
    batches: Mutex<Vec<RunBatch>>,
    requests: Mutex<Vec<ModelRequest>>,
    responses: Mutex<VecDeque<Result<ModelResponse, Error>>>,
    fail_append: Mutex<Option<usize>>,
    fail_final: Mutex<bool>,
    fail_gate: Mutex<Option<usize>>,
    gate_count: Mutex<usize>,
    handoff: Mutex<RunBatch>,
    rules: Mutex<Vec<GuardrailRule>>,
    pause: bool,
}
impl Fixture {
    fn event(&self, name: &str) {
        self.events.lock().unwrap().push(name.into());
    }
    fn options(self: &Arc<Self>) -> ChatLoopOptions {
        let agent = AgentConfig::new(
            "host-agent",
            ModelBinding::complete("offline", self.clone()),
        );
        let mut options = ChatLoopOptions::new(agent);
        options.session_store = Some(self.clone());
        options.config_source = Some(self.clone());
        options.tool_factory = Some(self.clone());
        options.trace_store = Some(self.clone());
        options.status = Some(self.clone());
        options
    }
    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}
fn error(message: &str) -> Error {
    Error::new(ErrorCategory::Host, message)
}
fn context() -> Context {
    Context {
        run_id: "host-test".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn item(text: &str, role: Role) -> RunItem {
    RunItem::Message {
        message: Message {
            role,
            content: vec![Content::Text { text: text.into() }],
        },
    }
}
fn response(items: Vec<RunItem>, end_turn: bool) -> ModelResponse {
    ModelResponse {
        snapshot_raw: None,
        snapshot_projection: None,
        raw: None,
        items,
        usage: Usage::default(),
        end_turn: Some(end_turn),
        response_id: None,
        metadata: Default::default(),
    }
}
impl Model for Fixture {
    fn provider(&self) -> &str {
        "offline"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        self.event("model");
        self.requests.lock().unwrap().push(request);
        Box::pin(async {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(response(vec![item("answer", Role::Assistant)], true)))
        })
    }
}
impl SessionStore for Fixture {
    fn working_state<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<WorkingState, Error>> {
        self.event("session.state");
        Box::pin(async { Ok(WorkingState::default()) })
    }
    fn load_messages<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Cursor,
        _: usize,
    ) -> BoxFuture<'a, Result<(Vec<UserMessage>, Cursor), Error>> {
        self.event("session.load");
        Box::pin(async {
            self.pages
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok((vec![], Cursor::default())))
        })
    }
    fn append_run_items<'a>(
        &'a self,
        _: &'a Context,
        batch: &'a RunBatch,
    ) -> BoxFuture<'a, Result<(), Error>> {
        self.event("session.append");
        self.batches.lock().unwrap().push(batch.clone());
        Box::pin(async {
            if *self.fail_append.lock().unwrap() == Some(self.batches.lock().unwrap().len()) {
                Err(error("append failure"))
            } else {
                Ok(())
            }
        })
    }
}
impl ConfigSource for Fixture {
    fn permission_mode<'a>(
        &'a self,
        _: &'a Context,
    ) -> BoxFuture<'a, Result<PermissionMode, Error>> {
        self.event("config.permission");
        Box::pin(async { Ok(PermissionMode::WorkspaceWrite) })
    }
    fn mode_directive<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<String, Error>> {
        self.event("config.directive");
        Box::pin(async { Ok("directive".into()) })
    }
    fn guardrail_rules<'a>(
        &'a self,
        _: &'a Context,
    ) -> BoxFuture<'a, Result<Vec<GuardrailRule>, Error>> {
        self.event("config.guardrails");
        Box::pin(async { Ok(self.rules.lock().unwrap().clone()) })
    }
    fn mode_snapshot<'a>(
        &'a self,
        _: &'a Context,
    ) -> BoxFuture<'a, Result<Option<builder::ModeSpec>, Error>> {
        self.event("config.mode");
        Box::pin(async { Ok(None) })
    }
    fn role_catalog<'a>(
        &'a self,
        _: &'a Context,
    ) -> BoxFuture<'a, Result<Vec<builder::RoleSpec>, Error>> {
        panic!("loop must not query roles")
    }
    fn handoff_history<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<RunBatch, Error>> {
        self.event("config.handoff");
        Box::pin(async { Ok(self.handoff.lock().unwrap().clone()) })
    }
}
impl PlatformToolFactory for Fixture {
    fn build_tools<'a>(
        &'a self,
        _: &'a Context,
        _: Vec<Arc<dyn Tool>>,
    ) -> BoxFuture<'a, Result<Vec<Arc<dyn Tool>>, Error>> {
        self.event("factory.build");
        Box::pin(async { Ok(vec![]) })
    }
}
impl TraceStore for Fixture {
    fn finalize<'a>(
        &'a self,
        _: &'a Context,
        _: &'a RunResult,
    ) -> BoxFuture<'a, Result<(), Error>> {
        self.event("trace.final");
        Box::pin(async {
            if *self.fail_final.lock().unwrap() {
                Err(error("finalize failure"))
            } else {
                Ok(())
            }
        })
    }
    fn run_dir<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<String, Error>> {
        panic!("not automatic")
    }
    fn append_category<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<(), Error>> {
        panic!("not automatic")
    }
    fn write_file<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
        _: &'a [u8],
    ) -> BoxFuture<'a, Result<(), Error>> {
        panic!("not automatic")
    }
}
impl RunStatusSink for Fixture {
    fn publish_final<'a>(
        &'a self,
        _: &'a Context,
        _: &'a RunResult,
    ) -> BoxFuture<'a, Result<(), Error>> {
        self.event("status.final");
        Box::pin(async { Ok(()) })
    }
    fn publish_progress<'a>(
        &'a self,
        _: &'a Context,
        _: &'a observability::ProgressSnapshot,
    ) -> BoxFuture<'a, Result<(), Error>> {
        panic!("not automatic")
    }
    fn publish_trace_id<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
    ) -> BoxFuture<'a, Result<(), Error>> {
        panic!("not automatic")
    }
}
fn page(id: i64, token: &str, texts: &[&str]) -> Result<(Vec<UserMessage>, Cursor), Error> {
    Ok((
        texts
            .iter()
            .map(|text| UserMessage {
                content: (*text).into(),
                ..Default::default()
            })
            .collect(),
        Cursor {
            message_id: id,
            token: token.into(),
        },
    ))
}
#[tokio::test]
async fn order_matches_real_go_oracle() {
    let fixture = Arc::new(Fixture::default());
    let mut loop_ = ChatLoop::new(fixture.options());
    loop_.run(context()).await.unwrap();
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/host-loop/sdk-chatloop.json"
    ))
    .unwrap();
    let case = oracle["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "preparation-order-and-handoff")
        .unwrap();
    let expected: Vec<String> = case["runs"][0]["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|call| call["op"].as_str().unwrap().to_owned())
        .take(8)
        .collect();
    assert_eq!(&fixture.events()[..8], expected);
    assert_eq!(
        &fixture.events()[8..],
        ["model", "session.append", "trace.final", "status.final"]
    );
}
#[tokio::test]
async fn cursor_pages_repeat_calls_and_authorship() {
    let fixture = Arc::new(Fixture::default());
    fixture.pages.lock().unwrap().extend([
        page(-7, "first", &[" "]),
        page(-7, "second", &["one"]),
        page(-8, "empty", &[]),
        page(-9, "next", &["two"]),
        page(-9, "next", &[]),
    ]);
    *fixture.handoff.lock().unwrap() = RunBatch {
        items: vec![item("old", Role::Assistant)],
        provenance: vec![ItemProvenance::Unknown],
        markers: vec![],
    };
    let mut options = fixture.options();
    options.message_limit = 1;
    let mut loop_ = ChatLoop::new(options);
    loop_.run(context()).await.unwrap();
    assert_eq!(
        loop_.cursor(),
        &Cursor {
            message_id: -8,
            token: "empty".into()
        }
    );
    loop_.run(context()).await.unwrap();
    let requests = fixture.requests.lock().unwrap();
    assert_eq!(
        requests[0].input,
        vec![item("old", Role::Assistant), item("one", Role::User)]
    );
    assert_eq!(
        requests[1].input,
        vec![item("old", Role::Assistant), item("two", Role::User)]
    );
    assert_eq!(
        requests[0].input_provenance,
        vec![ItemProvenance::Unknown, ItemProvenance::Unattributed]
    );
    assert_eq!(
        fixture.batches.lock().unwrap()[0].provenance,
        vec![ItemProvenance::Agent {
            name: "host-agent".into()
        }]
    );
}
#[tokio::test]
async fn later_page_failure_keeps_both_cursor_fields() {
    let fixture = Arc::new(Fixture::default());
    fixture
        .pages
        .lock()
        .unwrap()
        .extend([page(-11, "advanced", &["one"]), Err(error("load failure"))]);
    let mut options = fixture.options();
    options.message_limit = 1;
    let mut loop_ = ChatLoop::new(options);
    assert!(loop_.run(context()).await.err().unwrap().partial.is_none());
    assert_eq!(
        loop_.cursor(),
        &Cursor {
            message_id: -11,
            token: "advanced".into()
        }
    );
    assert!(fixture.requests.lock().unwrap().is_empty());
}
#[tokio::test]
async fn append_and_finalize_errors_retain_partial_and_stop_finalization() {
    for finalize in [false, true] {
        let fixture = Arc::new(Fixture::default());
        *fixture.fail_final.lock().unwrap() = finalize;
        if !finalize {
            *fixture.fail_append.lock().unwrap() = Some(1);
        }
        let failure = ChatLoop::new(fixture.options())
            .run(context())
            .await
            .err()
            .unwrap();
        assert_eq!(failure.partial.unwrap().new_items.len(), 1);
        assert!(!fixture.events().contains(&"status.final".into()));
        assert_eq!(fixture.events().contains(&"trace.final".into()), finalize);
    }
}
#[tokio::test]
async fn empty_success_appends_but_runner_failure_discards_partial() {
    let fixture = Arc::new(Fixture::default());
    fixture
        .responses
        .lock()
        .unwrap()
        .push_back(Ok(response(vec![], true)));
    ChatLoop::new(fixture.options())
        .run(context())
        .await
        .unwrap();
    assert!(fixture.batches.lock().unwrap()[0].items.is_empty());
    fixture.batches.lock().unwrap().clear();
    fixture.responses.lock().unwrap().extend([
        Ok(response(vec![item("partial", Role::Assistant)], false)),
        Err(Error::new(ErrorCategory::Provider, "failed")),
    ]);
    assert!(
        ChatLoop::new(fixture.options())
            .run(context())
            .await
            .err()
            .unwrap()
            .partial
            .is_none()
    );
    assert!(fixture.batches.lock().unwrap().is_empty());
}
#[tokio::test]
async fn ownership_and_caller_cancellation_are_independent() {
    let fixture = Arc::new(Fixture::default());
    let mut owner = builder::SessionState::new();
    let handle = owner.handle();
    let mut options = fixture.options();
    options.session = Session::Borrowed(handle.clone());
    let mut loop_ = ChatLoop::new(options);
    loop_.close().await.unwrap();
    assert!(!handle.is_closed());
    let token = CancellationToken::new();
    token.cancel();
    let mut cancelled = context();
    cancelled.cancellation = Arc::new(token);
    assert_eq!(
        loop_
            .run(cancelled)
            .await
            .err()
            .unwrap()
            .error
            .info
            .category,
        ErrorCategory::Cancelled
    );
    assert!(!handle.is_closed());
    loop_.run(context()).await.unwrap();
    owner.close().await.unwrap();
    assert_eq!(
        loop_
            .run(context())
            .await
            .err()
            .unwrap()
            .error
            .info
            .category,
        ErrorCategory::Cancelled
    );
    let owned = ChatLoop::new(fixture.options());
    let owned_handle = owned.session();
    drop(owned);
    assert!(owned_handle.is_closed());
}
#[tokio::test]
async fn invalid_rules_fail_before_mode_snapshot() {
    for rule in [
        GuardrailRule {
            regex: "[".into(),
            ..Default::default()
        },
        GuardrailRule {
            regex: "x".into(),
            action: "deny".into(),
            ..Default::default()
        },
        GuardrailRule {
            regex: "x".into(),
            rule_type: "unknown".into(),
            ..Default::default()
        },
    ] {
        let fixture = Arc::new(Fixture::default());
        fixture.rules.lock().unwrap().push(rule);
        assert!(
            ChatLoop::new(fixture.options())
                .run(context())
                .await
                .is_err()
        );
        assert_eq!(
            fixture.events(),
            ["config.permission", "config.directive", "config.guardrails"]
        );
    }
}
impl Tool for Fixture {
    fn definition(&self) -> &ToolDefinition {
        static DEFINITION: std::sync::LazyLock<ToolDefinition> =
            std::sync::LazyLock::new(|| ToolDefinition {
                name: "write".into(),
                description: String::new(),
                input_schema: serde_json::from_value(json!({"type":"object"})).unwrap(),
                read_only: false,
                requires_approval: true,
            });
        &DEFINITION
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        self.event("tool");
        Box::pin(async {
            Ok(ToolOutput {
                content: vec![Content::Text {
                    text: "written".into(),
                }],
                is_error: false,
                should_pause: self.pause,
            })
        })
    }
}
impl ApprovalGate for Fixture {
    fn approve_tool<'a>(
        &'a self,
        _: &'a Context,
        request: ToolApprovalRequest,
    ) -> BoxFuture<'a, Result<adk::runtime::compat::GoApprovalDecision, Error>> {
        assert_eq!(request.reason, "tool approval required");
        self.event("gate");
        Box::pin(async {
            let mut count = self.gate_count.lock().unwrap();
            *count += 1;
            if *self.fail_gate.lock().unwrap() == Some(*count) {
                Err(error("gate failure"))
            } else {
                Ok(adk::runtime::compat::GoApprovalDecision {
                    approved: true,
                    reason: String::new(),
                })
            }
        })
    }
}
impl RunHooks for Fixture {
    fn observe<'a>(
        &'a self,
        _: &'a Context,
        observation: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        if matches!(observation, Observation::CommittedItems { .. }) {
            self.event("user-hook");
        }
        Box::pin(async { Ok(()) })
    }
}
fn approval_options(fixture: &Arc<Fixture>) -> ChatLoopOptions {
    let mut options = fixture.options();
    options.tool_factory = None;
    options.approval_gate = Some(fixture.clone());
    options.agent.tools.push(fixture.clone());
    options.runner_config.hooks = Some(fixture.clone());
    fixture.responses.lock().unwrap().push_back(Ok(response(
        vec![
            RunItem::ToolCall {
                call: ToolCall {
                    id: "c1".into(),
                    name: "write".into(),
                    arguments: json!({}),
                },
            },
            RunItem::ToolCall {
                call: ToolCall {
                    id: "c2".into(),
                    name: "write".into(),
                    arguments: json!({}),
                },
            },
        ],
        false,
    )));
    options
}
#[tokio::test]
async fn approval_batch_is_committed_before_next_model_and_rebases_markers() {
    let fixture = Arc::new(Fixture::default());
    let result = ChatLoop::new(approval_options(&fixture))
        .run(context())
        .await
        .unwrap();
    assert!(result.result.pending_approvals.is_empty());
    let events = fixture.events();
    let models: Vec<_> = events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| (e == "model").then_some(i))
        .collect();
    let appends: Vec<_> = events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| (e == "session.append").then_some(i))
        .collect();
    assert_eq!(appends.len(), 3);
    assert!(appends[1] < models[1]);
    let batches = fixture.batches.lock().unwrap();
    assert_eq!(batches[0].markers.len(), 2);
    assert_eq!(batches[1].markers.len(), 2);
    assert_eq!(
        batches[1]
            .markers
            .iter()
            .map(|m| m.before_item)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(events.contains(&"user-hook".into()));
}
#[tokio::test]
async fn approval_append_failure_precedes_gate_failure_and_prevents_model() {
    let fixture = Arc::new(Fixture::default());
    *fixture.fail_gate.lock().unwrap() = Some(2);
    *fixture.fail_append.lock().unwrap() = Some(2);
    let failure = ChatLoop::new(approval_options(&fixture))
        .run(context())
        .await
        .err()
        .unwrap();
    assert!(failure.error.to_string().contains("append failure"));
    assert!(failure.partial.is_some());
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
    assert_eq!(fixture.batches.lock().unwrap()[1].markers.len(), 1);
}
#[tokio::test]
async fn model_failure_after_approval_discards_result_but_not_committed_effects() {
    let fixture = Arc::new(Fixture::default());
    let options = approval_options(&fixture);
    fixture
        .responses
        .lock()
        .unwrap()
        .push_back(Err(Error::new(ErrorCategory::Provider, "model failure")));
    let failure = ChatLoop::new(options).run(context()).await.err().unwrap();
    assert!(failure.partial.is_none());
    assert_eq!(fixture.batches.lock().unwrap().len(), 2);
}
struct BlockedStore;
impl SessionStore for BlockedStore {
    fn working_state<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<WorkingState, Error>> {
        Box::pin(std::future::pending())
    }
    fn load_messages<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Cursor,
        _: usize,
    ) -> BoxFuture<'a, Result<(Vec<UserMessage>, Cursor), Error>> {
        unreachable!()
    }
    fn append_run_items<'a>(
        &'a self,
        _: &'a Context,
        _: &'a RunBatch,
    ) -> BoxFuture<'a, Result<(), Error>> {
        unreachable!()
    }
}
#[tokio::test]
async fn cancellation_interrupts_pending_collaborators_without_tasks() {
    let fixture = Arc::new(Fixture::default());
    let mut options = fixture.options();
    options.session_store = Some(Arc::new(BlockedStore));
    let mut loop_ = ChatLoop::new(options);
    let token = CancellationToken::new();
    let mut ctx = context();
    ctx.cancellation = Arc::new(token.clone());
    let (result, ()) = tokio::join!(loop_.run(ctx), async {
        tokio::time::sleep(Duration::from_millis(5)).await;
        token.cancel();
    });
    assert_eq!(
        result.err().unwrap().error.info.category,
        ErrorCategory::Cancelled
    );
}

#[tokio::test]
async fn approved_pause_resolves_full_batch_without_next_model() {
    let fixture = Arc::new(Fixture {
        pause: true,
        ..Default::default()
    });
    let outcome = ChatLoop::new(approval_options(&fixture))
        .run(context())
        .await
        .unwrap();
    assert_eq!(outcome.result.status, RunStatus::Paused);
    assert!(outcome.result.pending_approvals.is_empty());
    assert_eq!(*fixture.gate_count.lock().unwrap(), 2);
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
    assert_eq!(fixture.batches.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn dynamic_rules_match_go_actions_and_multi_star_globs() {
    use adk::runtime::GuardrailInput;
    for action in ["", "block", " WaRN ", "log"] {
        let rule = GuardrailRule {
            name: "secret".into(),
            regex: "secret".into(),
            action: action.into(),
            rule_type: "tool-input".into(),
            tool_pattern: "*sql*_write".into(),
            message: " explicit ".into(),
        };
        let guards = compile_guardrail_rules(&[rule]).unwrap();
        let call = ToolCall {
            id: "c".into(),
            name: "mcp_sql_db_write".into(),
            arguments: json!({"text":"secret"}),
        };
        let result = guards.input[0]
            .check(&context(), "agent", GuardrailInput::ToolInput(&call))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            result.tripwire_triggered,
            action.is_empty() || action == "block"
        );
        assert_eq!(
            result.output,
            if action == "log" {
                serde_json::Value::Null
            } else {
                json!("explicit")
            }
        );
        let mismatch = ToolCall {
            name: "mcp_sql_db_read".into(),
            ..call
        };
        assert!(
            !guards.input[0]
                .check(&context(), "agent", GuardrailInput::ToolInput(&mismatch))
                .await
                .unwrap()
                .unwrap()
                .tripwire_triggered
        );
    }
    for regex in [r"\d", r"\D", r"\w", r"\W", r"\s", r"\S", r"\b"] {
        assert!(
            compile_guardrail_rules(&[GuardrailRule {
                name: "ascii".into(),
                regex: regex.into(),
                rule_type: "tool-input".into(),
                ..Default::default()
            }])
            .is_ok(),
            "{regex}"
        );
    }
    let result = compile_guardrail_rules(&[GuardrailRule {
        regex: r"\Qquoted\E".into(),
        rule_type: "tool-input".into(),
        ..Default::default()
    }]);
    assert!(result.err().unwrap().to_string().contains("Go/Rust regex"));
}

#[tokio::test]
async fn real_go_pagination_fixture_inputs_and_outputs() {
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/host-loop/sdk-chatloop.json"
    ))
    .unwrap();
    for name in [
        "default-50-drains-full-page",
        "pagination-blank-image-and-verbatim-payload",
        "full-page-stuck-cursor",
        "token-only-cursor-advances",
        "repeated-run-cursor-no-implicit-history",
    ] {
        let case = oracle["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        let spec = &case["spec"];
        let fixture = Arc::new(Fixture::default());
        let cursor = |v: &serde_json::Value| Cursor {
            message_id: v["message_id"].as_i64().unwrap_or(0),
            token: v["token"].as_str().unwrap_or("").into(),
        };
        for page in spec["Pages"].as_array().unwrap() {
            let messages = page["Messages"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|m| UserMessage {
                    id: m["ID"].as_i64().unwrap(),
                    content: m["Content"].as_str().unwrap().into(),
                    images: if m["Images"].is_null() {
                        vec![]
                    } else {
                        serde_json::from_value(m["Images"].clone()).unwrap()
                    },
                    ..Default::default()
                })
                .collect();
            fixture
                .pages
                .lock()
                .unwrap()
                .push_back(Ok((messages, cursor(&page["Next"]))));
        }
        let mut options = fixture.options();
        options.message_limit = spec["MessageLimit"].as_i64().unwrap();
        options.cursor = cursor(&spec["Cursor"]);
        let mut loop_ = ChatLoop::new(options);
        for (index, expected) in case["runs"].as_array().unwrap().iter().enumerate() {
            loop_.run(context()).await.unwrap();
            assert_eq!(loop_.cursor(), &cursor(&expected["cursor_after"]), "{name}");
            let model = expected["calls"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["op"] == "model.response")
                .unwrap();
            let requests = fixture.requests.lock().unwrap();
            let input = &requests[index].input;
            let wire = &model["args"]["input"].as_array().unwrap();
            assert_eq!(input.len(), wire.len(), "{name}");
            for (native, go) in input.iter().zip(wire.iter()) {
                let RunItem::Message { message } = native else {
                    panic!("expected message")
                };
                let text: String = message
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(text, go["message_text"].as_str().unwrap_or(""), "{name}");
                let images = message
                    .content
                    .iter()
                    .filter(|c| matches!(c, Content::Attachment { .. }))
                    .count();
                assert_eq!(
                    images,
                    go["message_images"].as_array().map_or(0, Vec::len),
                    "{name}: {go}"
                );
            }
        }
    }
}

#[tokio::test]
async fn no_gate_matches_oracle_denials_and_preserves_interrupted_history() {
    use adk::codec::approval::ApprovalPhase;
    let fixture = Arc::new(Fixture::default());
    let mut options = approval_options(&fixture);
    options.approval_gate = None;
    let outcome = ChatLoop::new(options).run(context()).await.unwrap();
    assert!(outcome.continuation.is_none());
    assert_eq!(outcome.result.pending_approvals.len(), 2);
    assert_eq!(outcome.result.history.len(), 2);
    assert_eq!(outcome.result.new_items.len(), 4);
    let batches = fixture.batches.lock().unwrap();
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[1].markers.len(), 2);
    assert!(
        batches[1]
            .markers
            .iter()
            .all(|m| m.marker.phase == ApprovalPhase::Denied && m.marker.agent.is_none())
    );
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/host-loop/sdk-chatloop.json"
    ))
    .unwrap();
    let case = oracle["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "batch-no-gate-denied-without-resume")
        .unwrap();
    let expected = &case["runs"][0]["result"];
    assert_eq!(expected["interrupted"], true);
    let expected_outputs: Vec<_> = expected["new_items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["type"] == "tool_output")
        .collect();
    for (native, go) in batches[1].items.iter().zip(expected_outputs) {
        let RunItem::ToolResult { output, .. } = native else {
            panic!("expected tool output")
        };
        assert!(output.is_error);
        assert_eq!(
            output.content,
            vec![Content::Text {
                text: go["tool_output"]["content"].as_str().unwrap().into()
            }]
        );
    }
    assert!(fixture.events().contains(&"trace.final".into()));
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn input_journal_is_replayed_without_duplicate_append_or_historic_execution() {
    use adk::codec::approval::{ApprovalMarker, ApprovalMarkerBoundary, ApprovalPhase};
    let fixture = Arc::new(Fixture::default());
    let call = ToolCall {
        id: "historic".into(),
        name: "write".into(),
        arguments: json!({}),
    };
    let input = RunBatch {
        items: vec![
            RunItem::ToolCall { call: call.clone() },
            RunItem::ToolResult {
                call_id: call.id.clone(),
                output: ToolOutput {
                    content: vec![Content::Text {
                        text: "denied".into(),
                    }],
                    is_error: true,
                    should_pause: false,
                },
            },
        ],
        provenance: vec![
            ItemProvenance::Agent {
                name: "old-agent".into()
            };
            2
        ],
        markers: [ApprovalPhase::Pending, ApprovalPhase::Denied]
            .into_iter()
            .map(|phase| ApprovalMarkerBoundary {
                before_item: 1,
                marker: ApprovalMarker::from_call(
                    &call,
                    phase,
                    Some(adk::codec::dto::AgentRef {
                        name: "old-agent".into(),
                    }),
                ),
            })
            .collect(),
    };
    *fixture.handoff.lock().unwrap() = input.clone();
    let result = ChatLoop::new(fixture.options())
        .run(context())
        .await
        .unwrap();
    assert_eq!(
        result.result.new_items,
        vec![item("answer", Role::Assistant)]
    );
    assert!(result.result.pending_approvals.is_empty());
    assert_eq!(&result.result.history[..2], &input.items);
    let requests = fixture.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].input, input.items);
    assert_eq!(requests[0].input_provenance, input.provenance);
    let batches = fixture.batches.lock().unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].items, result.result.new_items);
    assert!(batches[0].markers.is_empty());
    assert_eq!(*fixture.gate_count.lock().unwrap(), 0);
}

#[tokio::test]
async fn invalid_input_journal_fails_before_model() {
    use adk::codec::approval::{ApprovalMarker, ApprovalMarkerBoundary, ApprovalPhase};
    let fixture = Arc::new(Fixture::default());
    fixture
        .handoff
        .lock()
        .unwrap()
        .markers
        .push(ApprovalMarkerBoundary {
            before_item: 1,
            marker: ApprovalMarker::from_call(
                &ToolCall {
                    id: "call".into(),
                    name: "write".into(),
                    arguments: json!({}),
                },
                ApprovalPhase::Denied,
                None,
            ),
        });
    let failure = ChatLoop::new(fixture.options())
        .run(context())
        .await
        .err()
        .unwrap();
    assert_eq!(failure.error.info.category, ErrorCategory::InvalidInput);
    assert!(fixture.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn approval_resume_limit_uses_source_default_twelve() {
    for configured in [0, 1] {
        let fixture = Arc::new(Fixture::default());
        let mut options = approval_options(&fixture);
        options.max_resumes = configured;
        fixture.responses.lock().unwrap().clear();
        let limit = if configured == 0 { 12 } else { configured };
        for index in 0..=limit {
            fixture.responses.lock().unwrap().push_back(Ok(response(
                vec![RunItem::ToolCall {
                    call: ToolCall {
                        id: format!("c{index}"),
                        name: "write".into(),
                        arguments: json!({}),
                    },
                }],
                false,
            )));
        }
        let failure = ChatLoop::new(options).run(context()).await.err().unwrap();
        assert_eq!(failure.error.info.category, ErrorCategory::MaxTurns);
        assert!(failure.partial.is_some());
        assert_eq!(*fixture.gate_count.lock().unwrap(), limit as usize);
        assert_eq!(fixture.requests.lock().unwrap().len(), limit as usize + 1);
    }
}

#[tokio::test]
async fn collaborators_are_optional_and_factory_replaces_not_extends() {
    let fixture = Arc::new(Fixture::default());
    let mut options = ChatLoopOptions::new(AgentConfig::new(
        "optional",
        ModelBinding::complete("offline", fixture.clone()),
    ));
    options.agent.tools.push(fixture.clone());
    options.tool_factory = Some(fixture.clone());
    ChatLoop::new(options).run(context()).await.unwrap();
    assert!(fixture.requests.lock().unwrap()[0].tools.is_empty());
    assert_eq!(fixture.events(), ["factory.build", "model"]);
}

#[tokio::test]
async fn no_gate_append_failure_keeps_denials_and_skips_finalization() {
    let fixture = Arc::new(Fixture::default());
    *fixture.fail_append.lock().unwrap() = Some(2);
    let mut options = approval_options(&fixture);
    options.approval_gate = None;
    let failure = ChatLoop::new(options).run(context()).await.err().unwrap();
    assert!(
        failure
            .error
            .to_string()
            .contains("append denied approval items")
    );
    assert_eq!(failure.partial.unwrap().new_items.len(), 4);
    assert!(!fixture.events().contains(&"trace.final".into()));
}
