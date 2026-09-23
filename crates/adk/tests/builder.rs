#![cfg(feature = "builder")]
use adk::{
    builder::*,
    core::*,
    providers::{factory::Kind, routing::Routes},
    runtime::{CancellationToken, RunnerConfig, subagent::*},
};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
struct RecordingModel {
    requests: Mutex<Vec<ModelRequest>>,
}
fn response() -> ModelResponse {
    ModelResponse {
        raw: None,
        items: vec![RunItem::Message {
            message: Message {
                role: Role::Assistant,
                content: vec![Content::Text {
                    text: "offline".into(),
                }],
            },
        }],
        usage: Usage::default(),
        end_turn: Some(true),
        response_id: None,
        metadata: Default::default(),
    }
}
impl Model for RecordingModel {
    fn provider(&self) -> &str {
        "offline"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        self.requests.lock().unwrap().push(request);
        Box::pin(async { Ok(response()) })
    }
}
struct Events(VecDeque<ModelEvent>);
impl ModelStream for Events {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async { Ok(self.0.pop_front()) })
    }
}
impl StreamingModel for RecordingModel {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        self.requests.lock().unwrap().push(request);
        Box::pin(async {
            Ok(Box::new(Events(VecDeque::from([ModelEvent::Complete {
                response: response(),
            }]))) as Box<dyn ModelStream>)
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
        Box::pin(async { Ok(ApprovalDecision::Defer) })
    }
}
fn context() -> Context {
    Context {
        run_id: "builder-test".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn builder(config: Config, model: &Arc<RecordingModel>) -> Builder {
    Builder::new(config)
        .model("openai", Kind::OpenAi, model.clone())
        .unwrap()
}
fn category<T>(result: Result<T, Error>) -> ErrorCategory {
    result.err().expect("expected failure").info.category
}

#[tokio::test]
async fn defaults_build_and_run_offline_without_ambient_credentials_or_tools() {
    let model = Arc::new(RecordingModel::default());
    let mut bundle = builder(Config::default(), &model)
        .build(&context())
        .await
        .unwrap();
    assert_eq!(bundle.agent().name, "agent");
    assert_eq!(bundle.agent().model.name(), "gpt-5.6-sol");
    assert_eq!(bundle.policy().max_turns.get(), 100);
    assert_eq!(bundle.policy().tools.access, AccessMode::WorkspaceWrite);
    assert!(bundle.agent().tools.is_empty());
    assert_eq!(bundle.agent().settings["parallel_tool_calls"], true);
    assert_eq!(
        bundle
            .run(context(), vec![], Arc::new(TestHost))
            .await
            .unwrap()
            .result
            .status,
        RunStatus::Completed
    );
    assert_eq!(model.requests.lock().unwrap()[0].model, "gpt-5.6-sol");
    bundle.close().await.unwrap();
    bundle.close().await.unwrap();
}

#[tokio::test]
async fn strict_empty_overrides_legacy_tools_and_mode_instructions_not_access() {
    let model = Arc::new(RecordingModel::default());
    let mut config = Config {
        active_mode: Some("plan".into()),
        ..Default::default()
    };
    config.legacy_tools.enable_tools = true;
    config.legacy_tools.disable_default_tools = true;
    let mut legacy = builder(config.clone(), &model)
        .build(&context())
        .await
        .unwrap();
    assert_eq!(legacy.agent().tools.len(), 3);
    assert!(legacy.agent().instructions.contains("Read-Only Planning"));
    legacy.close().await.unwrap();
    config.features = Some(Features::default());
    let mut strict = builder(config, &model).build(&context()).await.unwrap();
    assert!(strict.agent().tools.is_empty());
    assert!(strict.agent().instructions.is_empty());
    assert_eq!(strict.policy().tools.access, AccessMode::ReadOnly);
    assert_eq!(strict.agent().settings["parallel_tool_calls"], false);
    strict.close().await.unwrap();
}

#[tokio::test]
async fn unavailable_and_unknown_features_fail_instead_of_producing_partial_bundle() {
    for feature in ["LSP", "Typo", "Bash"] {
        let model = Arc::new(RecordingModel::default());
        let config = Config {
            features: Some(Features {
                tools: [feature.into()].into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            category(builder(config, &model).build(&context()).await),
            ErrorCategory::InvalidInput
        );
        assert!(model.requests.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn tool_selection_and_dispatch_policy_match_under_plan_mode() {
    let model = Arc::new(RecordingModel::default());
    let config = Config {
        active_mode: Some("plan".into()),
        features: Some(Features {
            tools: ["ReadFile".into(), "Write".into(), "Signals.Finish".into()].into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut bundle = builder(config, &model).build(&context()).await.unwrap();
    let names: Vec<_> = bundle
        .agent()
        .tools
        .iter()
        .map(|t| t.definition().name.as_str())
        .collect();
    assert!(names.contains(&"read_file"));
    assert!(!names.contains(&"Write"));
    assert!(names.contains(&"finish"));
    assert_eq!(
        bundle.policy().tools.allowed_tools.as_ref().unwrap().len(),
        names.len()
    );
    bundle
        .run(context(), vec![], Arc::new(TestHost))
        .await
        .unwrap();
    let sent = model.requests.lock().unwrap()[0].tools.clone();
    assert_eq!(
        sent.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        names
    );
    bundle.close().await.unwrap();
}

#[tokio::test]
async fn mode_and_role_overrides_apply_in_order_and_only_narrow_access_and_turns() {
    let model = Arc::new(RecordingModel::default());
    let mode: ModeSpec = serde_json::from_value(json!({
        "name":"review", "instructions":"mode instructions", "toolAccess":"read-only",
        "constraints":{"maxTurns":5},
        "modelRouting": { "defaultModel":"medium", "settings":{"temperature":0.1},
            "fallbackModels":["openai/small"], "roleOverrides": { "critic": {
                "model":"large", "fallbackModels":[], "reasoningLevel":"high", "textVerbosity":"low", "settings":{"temperature":0.2}
            } }
        }
    })).unwrap();
    let config = Config {
        instructions: "host instructions".into(),
        mode_snapshot: Some(mode),
        active_role: Some("critic".into()),
        roles: vec![RoleSpec {
            name: "critic".into(),
            instructions: "role instructions".into(),
            tool_access: "full".into(),
            model_override: "small".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut bundle = builder(config, &model).build(&context()).await.unwrap();
    assert_eq!(bundle.agent().name, "critic");
    assert_eq!(bundle.agent().model.name(), "large");
    assert!(bundle.agent().fallbacks.is_empty());
    assert_eq!(bundle.agent().settings["temperature"], 0.2);
    assert_eq!(bundle.agent().settings["reasoning_effort"], "high");
    assert_eq!(bundle.agent().settings["text_verbosity"], "low");
    assert_eq!(bundle.policy().max_turns.get(), 5);
    assert_eq!(bundle.policy().tools.access, AccessMode::ReadOnly);
    assert!(
        bundle
            .agent()
            .instructions
            .starts_with("host instructions\n\nMode: review")
    );
    assert!(bundle.agent().instructions.ends_with("role instructions"));
    bundle
        .run(context(), vec![], Arc::new(TestHost))
        .await
        .unwrap();
    assert_eq!(model.requests.lock().unwrap()[0].model, "gpt-5.6-sol");
    bundle.close().await.unwrap();
}

#[tokio::test]
async fn route_prefixes_and_opaque_gateway_model_ids_are_preserved() {
    let model = Arc::new(RecordingModel::default());
    let mut routes = Routes::new("gateway");
    routes
        .register_kind("gateway", Kind::OpenRouter, model.clone())
        .unwrap();
    let config = Config {
        model: "gateway/anthropic/claude".into(),
        fallback_models: vec!["gateway/other/model".into()],
        ..Default::default()
    };
    let mut bundle = Builder::new(config)
        .routes(routes)
        .build(&context())
        .await
        .unwrap();
    bundle
        .stream(context(), vec![], Arc::new(TestHost))
        .finish()
        .await
        .unwrap();
    assert_eq!(model.requests.lock().unwrap()[0].model, "anthropic/claude");
    bundle.close().await.unwrap();
}

#[tokio::test]
async fn every_route_is_validated_before_tool_resource_configuration() {
    let model = Arc::new(RecordingModel::default());
    let config = Config {
        fallback_models: vec!["missing/model".into()],
        ..Default::default()
    };
    let root = tempfile::tempdir().unwrap();
    let expected = builder(config.clone(), &model)
        .build(&context())
        .await
        .err()
        .unwrap();
    let mut config = config;
    config.features = Some(Features {
        tools: ["Bash".into()].into(),
        ..Default::default()
    });
    let actual = builder(config, &model)
        .shell(adk_sandbox::Config::new(root.path().join("missing")))
        .build(&context())
        .await
        .err()
        .unwrap();
    assert_eq!(actual.info.category, ErrorCategory::InvalidInput);
    assert_eq!(actual.to_string(), expected.to_string());
    assert_eq!(
        category(Builder::new(Config::default()).build(&context()).await),
        ErrorCategory::InvalidInput
    );
}

#[test]
fn yaml_modes_crd_plain_yml_builtin_override_and_markdown_roles() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("modes")).unwrap();
    std::fs::create_dir(dir.path().join("agents")).unwrap();
    std::fs::write(dir.path().join("modes/chat.yaml"), "apiVersion: sdk/v1\nkind: ModeTemplate\nmetadata:\n  name: chat\nspec:\n  displayName: Custom chat\n  toolAccess: read_only\n  instructions: Inspect only\n").unwrap();
    std::fs::write(dir.path().join("modes/review.yml"), "displayName: Review\nmodelRouting:\n  defaultModel: openai/small\nconstraints:\n  maxTurns: 4\n").unwrap();
    std::fs::write(dir.path().join("agents/critic.md"), "---\r\nname: critic\r\nmodel: openai/small\r\ntoolAccess: readonly\r\n---\r\nCritique the design.\r\n").unwrap();
    let source = FileConfigSource::new(dir.path());
    let config = source.load_files(&context()).unwrap();
    assert_eq!(
        config
            .modes
            .iter()
            .map(|m| m.name.as_str())
            .collect::<Vec<_>>(),
        ["chat", "plan", "review"]
    );
    assert_eq!(config.modes[0].display_name, "Custom chat");
    assert_eq!(config.modes[2].version, "v1");
    assert_eq!(config.roles[0].instructions, "Critique the design.");
    assert_eq!(config.roles[0].model_override, "openai/small");
}

#[test]
fn missing_directories_use_builtins_but_bad_config_is_not_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let source = FileConfigSource::new(dir.path());
    assert_eq!(source.load_files(&context()).unwrap().modes.len(), 2);
    std::fs::create_dir(dir.path().join("modes")).unwrap();
    for text in [
        "instructions: [broken",
        "name: ../escape",
        "toolAccess: typo",
        "constraints:\n  maxTurns: 0",
        "unknownFeature: true",
    ] {
        std::fs::write(dir.path().join("modes/bad.yaml"), text).unwrap();
        assert_eq!(
            category(source.load_files(&context())),
            ErrorCategory::InvalidInput
        );
    }
}

#[tokio::test]
async fn file_source_is_used_for_active_mode_and_role() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("agents")).unwrap();
    std::fs::write(dir.path().join("agents/critic.md"), "Review safely.").unwrap();
    let config = Config {
        active_mode: Some("PLAN".into()),
        active_role: Some("critic".into()),
        ..Default::default()
    };
    let model = Arc::new(RecordingModel::default());
    let mut bundle = builder(config, &model)
        .source(Arc::new(FileConfigSource::new(dir.path())))
        .build(&context())
        .await
        .unwrap();
    assert!(bundle.agent().instructions.ends_with("Review safely."));
    assert_eq!(bundle.policy().tools.access, AccessMode::ReadOnly);
    bundle.close().await.unwrap();
}

#[tokio::test]
async fn shared_session_survives_bundle_close_and_rebuild_but_owner_close_revokes_streams() {
    let model = Arc::new(RecordingModel::default());
    let mut state = SessionState::new();
    let mut first = builder(Config::default(), &model)
        .session(state.handle())
        .build(&context())
        .await
        .unwrap();
    first.close().await.unwrap();
    assert!(!state.handle().is_closed());
    let mut second = builder(Config::default(), &model)
        .session(state.handle())
        .build(&context())
        .await
        .unwrap();
    second
        .run(context(), vec![], Arc::new(TestHost))
        .await
        .unwrap();
    let stream = second.stream(context(), vec![], Arc::new(TestHost));
    state.close().await.unwrap();
    state.close().await.unwrap();
    assert_eq!(
        stream.finish().await.err().unwrap().error.info.category,
        ErrorCategory::Cancelled
    );
    assert_eq!(
        category(
            builder(Config::default(), &model)
                .session(state.handle())
                .build(&context())
                .await
        ),
        ErrorCategory::Cancelled
    );
    second.close().await.unwrap();
}

#[tokio::test]
async fn owned_bundle_close_and_drop_revoke_escaped_streams_and_session_handles() {
    let model = Arc::new(RecordingModel::default());
    for explicit in [true, false] {
        let mut bundle = builder(Config::default(), &model)
            .build(&context())
            .await
            .unwrap();
        let session = bundle.session();
        let stream = bundle.stream(context(), vec![], Arc::new(TestHost));
        if explicit {
            bundle.close().await.unwrap();
        }
        drop(bundle);
        assert!(session.is_closed());
        assert_eq!(
            stream.finish().await.err().unwrap().error.info.category,
            ErrorCategory::Cancelled
        );
    }
}

#[tokio::test]
async fn cancellation_and_expired_deadlines_prevent_construction() {
    let model = Arc::new(RecordingModel::default());
    let token = CancellationToken::new();
    token.cancel();
    let mut context = Context {
        cancellation: Arc::new(token),
        ..context()
    };
    assert_eq!(
        category(builder(Config::default(), &model).build(&context).await),
        ErrorCategory::Cancelled
    );
    context.cancellation = Arc::new(CancellationToken::new());
    context.deadline = Some(std::time::Instant::now() - Duration::from_secs(1));
    assert_eq!(
        category(builder(Config::default(), &model).build(&context).await),
        ErrorCategory::DeadlineExceeded
    );
}

struct PendingChild;
impl ChildExecutor for PendingChild {
    fn execute<'a>(
        &'a self,
        _: ChildInvocation,
        _: ChildControl,
    ) -> BoxFuture<'a, Result<ChildOutcome, Error>> {
        Box::pin(std::future::pending())
    }
}
#[tokio::test]
async fn owned_scheduler_is_joined_on_build_failure_and_shared_scheduler_survives_rebuild() {
    let model = Arc::new(RecordingModel::default());
    let scheduler = Scheduler::new(
        context(),
        SchedulerConfig::default(),
        Arc::new(PendingChild),
        None,
    )
    .unwrap();
    let state = SessionState::with_scheduler(scheduler);
    let handle = state.handle();
    let result = builder(Config::default(), &model)
        .owned_session(state)
        .runner_config(RunnerConfig {
            limits: adk::runtime::Limits {
                max_cost: Some(-1.0),
                ..Default::default()
            },
            ..Default::default()
        })
        .build(&context())
        .await;
    assert_eq!(category(result), ErrorCategory::InvalidInput);
    assert!(handle.is_closed());
    let scheduler = Scheduler::new(
        context(),
        SchedulerConfig::default(),
        Arc::new(PendingChild),
        None,
    )
    .unwrap();
    let mut state = SessionState::with_scheduler(scheduler);
    let config = Config {
        features: Some(Features {
            subagents: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut first = builder(config.clone(), &model)
        .session(state.handle())
        .build(&context())
        .await
        .unwrap();
    assert!(
        first
            .agent()
            .tools
            .iter()
            .any(|t| t.definition().name == "subagent")
    );
    let session = state.handle().subagents().unwrap().clone();
    first.close().await.unwrap();
    let mut second = builder(config, &model)
        .session(state.handle())
        .build(&context())
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&session, second.session().subagents().unwrap()));
    second.close().await.unwrap();
    state.close().await.unwrap();
}

#[tokio::test]
async fn subagents_require_an_owned_session_scheduler() {
    let model = Arc::new(RecordingModel::default());
    let config = Config {
        features: Some(Features {
            subagents: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        category(builder(config, &model).build(&context()).await),
        ErrorCategory::InvalidInput
    );
}

#[test]
fn default_file_source_requires_absolute_home() {
    const CHILD: &str = "ADK_BUILDER_HOME_TEST";
    if let Some(case) = std::env::var_os(CHILD) {
        let source = FileConfigSource::default();
        if case == "absolute" {
            assert_eq!(source.load_files(&context()).unwrap().modes.len(), 2);
        } else {
            assert_eq!(
                category(source.load_files(&context())),
                ErrorCategory::InvalidInput
            );
        }
        let explicit = FileConfigSource::new(".gratefulagents");
        let loaded = explicit.load_files(&context()).unwrap();
        assert_eq!(
            loaded
                .modes
                .iter()
                .find(|mode| mode.name == "plan")
                .unwrap()
                .tool_access,
            "full-access"
        );
        return;
    }
    let repository = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(repository.path().join(".gratefulagents/modes")).unwrap();
    std::fs::write(
        repository.path().join(".gratefulagents/modes/plan.yaml"),
        "name: plan\ntoolAccess: full-access\n",
    )
    .unwrap();
    for case in ["absent", "empty", "relative", "absolute"] {
        let mut command = std::process::Command::new(
            std::fs::canonicalize(std::env::args_os().next().unwrap()).unwrap(),
        );
        command
            .args([
                "--exact",
                "default_file_source_requires_absolute_home",
                "--nocapture",
            ])
            .current_dir(repository.path())
            .env(CHILD, case)
            .env_remove("HOME");
        match case {
            "empty" => {
                command.env("HOME", "");
            }
            "relative" => {
                command.env("HOME", ".");
            }
            "absolute" => {
                command.env("HOME", home.path());
            }
            _ => {}
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{case}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[tokio::test]
async fn shell_input_does_not_enable_unselected_tools() {
    let model = Arc::new(RecordingModel::default());
    let root = tempfile::tempdir().unwrap();
    let config = Config {
        features: Some(Features::default()),
        ..Default::default()
    };
    let mut bundle = builder(config, &model)
        .shell(adk_sandbox::Config::new(root.path().join("missing")))
        .build(&context())
        .await
        .unwrap();
    assert!(bundle.agent().tools.is_empty());
    bundle.close().await.unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct UnusedBrowser;
#[cfg(any(target_os = "linux", target_os = "macos"))]
impl adk_tools::browser::Runner for UnusedBrowser {
    fn run<'a>(
        &'a self,
        _: &'a ToolContext,
        _: adk_tools::browser::Request,
    ) -> BoxFuture<'a, Result<adk_tools::browser::Execution, String>> {
        panic!("construction must not run the browser")
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn typed_resources_preserve_selection_and_plan_policy() {
    let root = tempfile::tempdir().unwrap();
    let model = Arc::new(RecordingModel::default());
    for selected in [false, true] {
        let mut sandbox = adk_sandbox::Config::new(root.path());
        sandbox.backend = adk_sandbox::Backend::Local;
        let executor = Arc::new(adk_sandbox::Executor::new(sandbox.clone()).unwrap());
        let config = Config {
            active_mode: Some("plan".into()),
            tool_options: adk_tools::Config {
                allow_private_network_urls: true,
                ..Default::default()
            },
            features: Some(Features {
                tools: if selected {
                    ["Bash".into(), "LSP".into(), "Browser".into()].into()
                } else {
                    Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut bundle = builder(config, &model)
            .shell(sandbox)
            .lsp(adk_tools::lsp::Config {
                executor,
                servers: vec![],
                discoverer: None,
            })
            .browser(adk_tools::browser::Config {
                runner: Arc::new(UnusedBrowser),
                executable: selected.then(|| root.path().join("chrome")),
                screenshot_dir: root.path().into(),
                access: AccessMode::FullAccess,
                allow_private_network_urls: true,
            })
            .build(&context())
            .await
            .unwrap();
        assert_eq!(bundle.policy().tools.access, AccessMode::ReadOnly);
        assert_eq!(bundle.agent().tools.len(), if selected { 3 } else { 0 });
        assert_eq!(
            bundle.policy().tools.allowed_tools.as_ref().unwrap().len(),
            bundle.agent().tools.len()
        );
        bundle.close().await.unwrap();
    }
}
