#![cfg(feature = "builder")]
use adk::{
    builder::{Builder, Config, Features},
    core::*,
    guardrails::*,
    providers::factory::Kind,
    runtime::*,
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

fn context() -> Context {
    Context {
        run_id: "builtin-test".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        raw_arguments: None,
        id: "call".into(),
        name: name.into(),
        arguments,
    }
}
fn token() -> String {
    ["gh", "p_", &"a".repeat(36)].concat()
}
fn partial() -> String {
    ["AK", "IA", &"A".repeat(16)].concat()
}
fn output(text: String) -> ToolOutput {
    ToolOutput {
        content: vec![Content::Text { text }],
        is_error: false,
        should_pause: false,
    }
}

#[tokio::test]
async fn direct_input_names_typed_arguments_and_narrow_shell_policy() {
    let guards = builtin_tool_input_guardrails();
    assert_eq!(
        guards.iter().map(|g| g.name()).collect::<Vec<_>>(),
        ["block-destructive-commands", "detect-secret-leak"]
    );
    for (name, args, blocked) in [
        ("Bash", json!({"command":"cargo test"}), false),
        ("custom_shell", json!({"cmd":"git status"}), false),
        ("execute", json!({"command":"rm -fr /"}), true),
        ("execute", json!({"cmd":"sudo -u root rm -rf /"}), true),
        (
            "Bash",
            json!({"command":"echo safe", "cmd":"rm -rf /"}),
            false,
        ),
        ("Bash", json!({"command":"", "cmd":"rm -rf /"}), true),
        ("Bash", json!({}), false),
        ("Bash", json!({"command":3}), true),
        ("Bash", json!({"command":"echo safe", "cmd":3}), true),
        ("Bash", json!({"command":null}), false),
        ("Bash", json!({"command":null, "cmd":"rm -rf /"}), true),
        ("Bash", json!([]), true),
        ("Bash", json!(null), false),
        ("reader", json!({"command":"rm -rf /"}), false),
        ("Bash", json!({"command":"echo 'unfinished"}), true),
    ] {
        let checked = run_guardrails(
            &guards,
            &context(),
            "a",
            GuardrailInput::ToolInput(&call(name, args)),
        )
        .await;
        assert_eq!(checked.tripped(), blocked, "{name}");
    }
    let secret = token();
    let checked = run_guardrails(
        &guards,
        &context(),
        "a",
        GuardrailInput::ToolInput(&call("reader", json!({"nested":[secret]}))),
    )
    .await;
    assert!(checked.tripped());
    assert_eq!(
        checked.reports.last().unwrap().guardrail_name,
        "detect-secret-leak"
    );
    assert!(
        !serde_json::to_string(&checked.reports)
            .unwrap()
            .contains(&secret)
    );
    for guard in guards.into_iter().chain(builtin_tool_output_guardrails()) {
        assert_eq!(
            guard.durable_key().unwrap(),
            format!("adk.builtin.{}.v1", guard.name())
        );
        assert!(
            guard
                .check(&context(), "a", GuardrailInput::Input(&[]))
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn output_redaction_and_partial_block_never_put_secrets_in_reports() {
    let guards = builtin_tool_output_guardrails();
    let call = call("reader", json!({}));
    for (secret, blocked) in [
        (token(), false),
        (partial(), true),
        (["{\"type\":\"service_", "account\"}"].concat(), true),
    ] {
        let mut result = output(format!("prefix {secret} suffix"));
        result.is_error = true;
        result.should_pause = true;
        let checked =
            run_tool_output_guardrails(&guards, &context(), "a", &call, &mut result).await;
        assert_eq!(checked.tripped(), blocked);
        assert!(
            !serde_json::to_string(&checked.reports)
                .unwrap()
                .contains(&secret)
        );
        assert!(result.is_error && result.should_pause);
        if !blocked {
            let Content::Text { text } = &result.content[0] else {
                panic!("expected text")
            };
            assert!(!text.contains(&secret));
            assert!(
                text.contains("prefix")
                    && text.contains("suffix")
                    && text.contains("[REDACTED:")
                    && text.contains("do not try to reconstruct")
            );
        }
    }
    let mut mixed = output("ordinary text".into());
    mixed.content.push(Content::Image {
        uri: "image://fixture".into(),
        media_type: "image/png".into(),
    });
    let original = mixed.clone();
    let checked = run_tool_output_guardrails(&guards, &context(), "a", &call, &mut mixed).await;
    assert!(checked.error.is_none());
    assert_eq!(mixed, original);
    mixed.content[0] = Content::Text { text: token() };
    let checked = run_tool_output_guardrails(&guards, &context(), "a", &call, &mut mixed).await;
    assert!(checked.tripped());
    assert!(matches!(&mixed.content[1], Content::Image { .. }));
    assert!(
        !serde_json::to_string(&checked.reports)
            .unwrap()
            .contains(&token())
    );
}

struct Script(Mutex<VecDeque<ModelResponse>>);
fn response(items: Vec<RunItem>) -> ModelResponse {
    let end_turn = Some(!items.iter().any(|i| matches!(i, RunItem::ToolCall { .. })));
    ModelResponse {
        items,
        end_turn,
        usage: Usage::default(),
        response_id: None,
        metadata: Default::default(),
        raw: None,
        snapshot_raw: None,
        snapshot_projection: None,
    }
}
impl Model for Script {
    fn provider(&self) -> &str {
        "offline"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async {
            Ok(self
                .0
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| response(vec![])))
        })
    }
}
struct Events(Option<ModelResponse>);
impl ModelStream for Events {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async {
            Ok(self
                .0
                .take()
                .map(|response| ModelEvent::Complete { response }))
        })
    }
}
impl StreamingModel for Script {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async {
            Ok(Box::new(Events(Some(
                self.0
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| response(vec![])),
            ))) as Box<dyn ModelStream>)
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
        Box::pin(async { Ok(ApprovalDecision::Approve) })
    }
}
struct Probe {
    definition: ToolDefinition,
    calls: AtomicUsize,
    result: ToolOutput,
}
impl Tool for Probe {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(self.result.clone()) })
    }
}
struct Custom {
    name: &'static str,
    seen: Arc<Mutex<Vec<String>>>,
}
impl Guardrail for Custom {
    fn name(&self) -> &str {
        self.name
    }
    fn check<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
        input: GuardrailInput<'a>,
    ) -> BoxFuture<'a, Result<Option<GuardrailResult>, Error>> {
        Box::pin(async move {
            let value = match input {
                GuardrailInput::ToolOutput { output, .. } => serde_json::to_string(output).unwrap(),
                _ => self.name.into(),
            };
            self.seen.lock().unwrap().push(value);
            Ok(None)
        })
    }
}

#[tokio::test]
async fn builder_real_execution_gate_and_builtin_before_custom() {
    for (explicit, legacy, enabled) in [
        (None, false, false),
        (None, true, true),
        (Some(false), true, false),
        (Some(true), false, true),
    ] {
        for destructive in [false, true] {
            let seen = Arc::new(Mutex::new(vec![]));
            let probe = Arc::new(Probe {
                definition: ToolDefinition {
                    name: "custom_shell".into(),
                    description: "probe".into(),
                    input_schema: json!({"type":"object"}).try_into().unwrap(),
                    read_only: true,
                    requires_approval: false,
                },
                calls: AtomicUsize::new(0),
                result: output(token()),
            });
            let model = Arc::new(Script(Mutex::new(VecDeque::from([response(vec![
                RunItem::ToolCall {
                    call: call(
                        "custom_shell",
                        json!({"command":if destructive { "rm -fr /" } else { "cargo test" }}),
                    ),
                },
            ])]))));
            let config = Config {
                model: "openai/test".into(),
                enable_guardrails: legacy,
                legacy_tools: adk::tools::LegacyFeatures {
                    enable_tools: true,
                    disable_default_tools: true,
                    disable_signal_tools: true,
                    disable_web_tools: true,
                    ..Default::default()
                },
                features: explicit.map(|builtin_guardrails| Features {
                    builtin_guardrails,
                    tools: ["ExtraTools".into()].into(),
                    ..Default::default()
                }),
                ..Default::default()
            };
            assert_eq!(config.resolved_features().builtin_guardrails, enabled);
            let runner = RunnerConfig {
                tool_input_guardrails: vec![Arc::new(Custom {
                    name: "custom-input",
                    seen: seen.clone(),
                })],
                tool_output_guardrails: vec![Arc::new(Custom {
                    name: "custom-output",
                    seen: seen.clone(),
                })],
                ..Default::default()
            };
            let mut bundle = Builder::new(config)
                .model("openai", Kind::OpenAi, model)
                .unwrap()
                .extra_tools([probe.clone() as Arc<dyn Tool>])
                .runner_config(runner)
                .build(&context())
                .await
                .unwrap();
            let result = bundle.run(context(), vec![], Arc::new(TestHost)).await;
            let blocked = enabled && destructive;
            assert_eq!(probe.calls.load(Ordering::SeqCst), usize::from(!blocked));
            if blocked {
                assert!(seen.lock().unwrap().is_empty());
            } else {
                let result = result.unwrap();
                let reports: Vec<_> = result
                    .result
                    .guardrails
                    .iter()
                    .map(|r| r.guardrail_name.as_str())
                    .collect();
                assert_eq!(
                    reports,
                    if enabled {
                        vec![
                            "block-destructive-commands",
                            "detect-secret-leak",
                            "custom-input",
                            "detect-secret-in-output",
                            "custom-output",
                        ]
                    } else {
                        vec!["custom-input", "custom-output"]
                    }
                );
                let seen = seen.lock().unwrap();
                assert_eq!(seen.len(), 2);
                assert_eq!(seen[1].contains(&token()), !enabled);
                assert_eq!(seen[1].contains("[REDACTED:"), enabled);
            }
            bundle.close().await.unwrap();
        }
    }
}

#[derive(Default)]
struct Store(Mutex<Vec<RunnerCheckpoint>>);
impl CheckpointStore for Store {
    fn persist<'a>(
        &'a self,
        _: &'a Context,
        checkpoint: &'a RunnerCheckpoint,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(checkpoint.clone());
            Ok(())
        })
    }
}
#[tokio::test]
async fn builtin_guards_support_durable_execution_and_policy_identity() {
    let model = Arc::new(Script(Mutex::new(VecDeque::from([response(vec![])]))));
    let agent = AgentConfig::new("agent", ModelBinding::complete("model", model));
    let runner = Runner::new(
        agent.clone(),
        RunnerConfig {
            tool_input_guardrails: builtin_tool_input_guardrails(),
            tool_output_guardrails: builtin_tool_output_guardrails(),
            ..Default::default()
        },
    )
    .unwrap();
    let store = Arc::new(Store::default());
    let request = RunRequest {
        input: vec![],
        input_provenance: vec![],
        policy: RunPolicy::default(),
    };
    runner
        .run_durable(
            context(),
            request.clone(),
            Arc::new(TestHost),
            DurableRun::new(store.clone()),
        )
        .await
        .unwrap();
    let checkpoint = store.0.lock().unwrap().last().unwrap().clone();
    let resumed = DurableRun {
        resume: Some(checkpoint.clone()),
        ..DurableRun::new(store.clone())
    };
    runner
        .run_durable(context(), request.clone(), Arc::new(TestHost), resumed)
        .await
        .unwrap();
    let disabled = Runner::new(agent, RunnerConfig::default()).unwrap();
    let resumed = DurableRun {
        resume: Some(checkpoint),
        ..DurableRun::new(store)
    };
    let error = disabled
        .run_durable(context(), request, Arc::new(TestHost), resumed)
        .await
        .err()
        .expect("guardrail configuration must be bound to checkpoint");
    assert_eq!(error.error.info.category, ErrorCategory::Unsupported);
}

struct RecordingModel {
    script: Script,
    requests: Mutex<Vec<ModelRequest>>,
}
impl Model for RecordingModel {
    fn provider(&self) -> &str {
        "offline"
    }
    fn complete<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        self.requests.lock().unwrap().push(request.clone());
        self.script.complete(context, request)
    }
}
#[derive(Default)]
struct RecordingObserver {
    outputs: Mutex<Vec<ToolOutput>>,
    events: Mutex<Vec<RunEvent>>,
}
impl RunHooks for RecordingObserver {
    fn observe<'a>(
        &'a self,
        _: &'a Context,
        observation: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        if let Observation::RawToolOutput { output, .. } = observation {
            self.outputs.lock().unwrap().push(output);
        }
        Box::pin(async { Ok(()) })
    }
}
impl Host for RecordingObserver {
    fn emit<'a>(&'a self, _: &'a Context, event: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        self.events.lock().unwrap().push(event);
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

#[tokio::test]
async fn runner_blocks_multipart_credentials_before_model_history_and_observations() {
    for secret in [
        token(),
        partial(),
        ["{\"type\":\"service_", "account\"}"].concat(),
    ] {
        for content in [
            vec![Content::Reasoning {
                text: format!("prefix {secret} suffix"),
                signature: None,
            }],
            vec![
                Content::Text {
                    text: "ordinary text".into(),
                },
                Content::Reasoning {
                    text: format!("prefix {secret} suffix"),
                    signature: None,
                },
            ],
            vec![
                Content::Reasoning {
                    text: "x".into(),
                    signature: None,
                },
                Content::Text {
                    text: secret.clone(),
                },
            ],
            vec![
                Content::Text { text: "x".into() },
                Content::Text {
                    text: secret.clone(),
                },
            ],
            vec![
                Content::Text {
                    text: secret[..4].into(),
                },
                Content::Text {
                    text: secret[4..].into(),
                },
            ],
        ] {
            let probe = Arc::new(Probe {
                definition: ToolDefinition {
                    name: "reader".into(),
                    description: "synthetic output probe".into(),
                    input_schema: json!({"type":"object"}).try_into().unwrap(),
                    read_only: true,
                    requires_approval: false,
                },
                calls: AtomicUsize::new(0),
                result: ToolOutput {
                    content,
                    is_error: false,
                    should_pause: false,
                },
            });
            let model = Arc::new(RecordingModel {
                script: Script(Mutex::new(VecDeque::from([
                    response(vec![RunItem::ToolCall {
                        call: call("reader", json!({})),
                    }]),
                    response(vec![]),
                ]))),
                requests: Mutex::new(vec![]),
            });
            let observer = Arc::new(RecordingObserver::default());
            let mut agent =
                AgentConfig::new("agent", ModelBinding::complete("model", model.clone()));
            agent.tools = vec![probe.clone()];
            agent.hooks = Some(observer.clone());
            let runner = Runner::new(
                agent,
                RunnerConfig {
                    tool_output_guardrails: builtin_tool_output_guardrails(),
                    ..Default::default()
                },
            )
            .unwrap();
            let outcome = runner
                .run(
                    context(),
                    RunRequest {
                        input: vec![],
                        input_provenance: vec![],
                        policy: RunPolicy::default(),
                    },
                    observer.clone(),
                )
                .await
                .unwrap();
            assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
            assert_eq!(model.requests.lock().unwrap().len(), 2);
            assert_eq!(observer.outputs.lock().unwrap().len(), 1);
            assert!(!observer.events.lock().unwrap().is_empty());
            assert!(
                outcome
                    .result
                    .guardrails
                    .iter()
                    .any(|report| report.guardrail_name == "detect-secret-in-output"
                        && report.tripwire_triggered)
            );
            assert!(
                outcome.result.history.iter().any(
                    |item| matches!(item, RunItem::ToolResult { output, .. } if output.is_error)
                )
            );
            let escaped = serde_json::to_string(&secret).unwrap();
            for visible in [
                serde_json::to_string(&*model.requests.lock().unwrap()).unwrap(),
                serde_json::to_string(&outcome.result).unwrap(),
                serde_json::to_string(&*observer.outputs.lock().unwrap()).unwrap(),
                serde_json::to_string(&*observer.events.lock().unwrap()).unwrap(),
            ] {
                assert!(
                    !visible.contains(&secret),
                    "credential escaped output protection"
                );
                assert!(
                    !visible.contains(&escaped[1..escaped.len() - 1]),
                    "encoded credential escaped output protection"
                );
            }
        }
    }
}
