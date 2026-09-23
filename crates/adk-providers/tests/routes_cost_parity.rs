use adk_core::*;
use adk_providers::{
    auth::{AuthMode, CredentialStore, Material, Refresh, Scope, Secret, Session},
    cost,
    factory::{DEFAULT_CHAT_MODEL, Kind, RouteSpec, default_route, supports_chat_completions},
    metadata::{
        ModelMetadata, fetch_model_metadata, model_metadata_by_id, model_metadata_endpoint,
        parse_model_metadata, picker_model_metadata,
    },
    routing::Routes,
    wire::{self, Protocol},
};
use adk_runtime::CancellationToken;
use serde_json::json;
use std::sync::{Arc, Mutex};

fn context() -> Context {
    Context {
        run_id: "routes-fixture".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn request(model: &str) -> ModelRequest {
    ModelRequest {
        input_provenance: Vec::new(),
        model: model.into(),
        instructions: String::new(),
        input: vec![],
        tools: vec![],
        output_schema: None,
        output_schema_name: "output".into(),
        output_schema_strict: true,
        settings: Default::default(),
    }
}
fn usage() -> Usage {
    Usage {
        requests: 1,
        input_tokens: 1000,
        output_tokens: 100,
        cache_read_tokens: 200,
        cache_creation_tokens: 50,
        context_tokens: None,
    }
}
fn answer() -> ModelResponse {
    ModelResponse {
        snapshot_raw: None,
        raw: None,
        items: vec![RunItem::Message {
            message: Message {
                role: Role::Assistant,
                content: vec![Content::Text {
                    text: "fixture answer".into(),
                }],
            },
        }],
        usage: usage(),
        end_turn: Some(true),
        response_id: None,
        metadata: Default::default(),
    }
}
#[derive(Default)]
struct Probe {
    requests: Mutex<Vec<String>>,
    fail: bool,
}
impl Model for Probe {
    fn provider(&self) -> &str {
        "fixture"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request.model);
            if self.fail {
                Err(adk_providers::error::RequestFailure::http(
                    429,
                    &Default::default(),
                    std::time::SystemTime::UNIX_EPOCH,
                )
                .into_error())
            } else {
                Ok(answer())
            }
        })
    }
}
struct Events(Option<ModelResponse>);
impl ModelStream for Events {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async move {
            Ok(self
                .0
                .take()
                .map(|response| ModelEvent::Complete { response }))
        })
    }
}
impl StreamingModel for Probe {
    fn stream<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            Ok(
                Box::new(Events(Some(self.complete(context, request).await?)))
                    as Box<dyn ModelStream>,
            )
        })
    }
}
fn near(got: f64, expected: f64) {
    assert!(
        (got - expected).abs() < 1e-12,
        "got {got}, expected {expected}"
    );
}

#[test]
fn default_precedence_scopes_aliases_and_protocols_match_pinned_baseline() {
    for (explicit, model, configured, expected) in [
        (
            Some(" TEAM "),
            "anthropic/large",
            Some(Kind::OpenAi),
            "team",
        ),
        (None, " Anthropic/large ", Some(Kind::OpenAi), "anthropic"),
        (Some(" "), "large", Some(Kind::Anthropic), "anthropic"),
        (None, "large", None, "openai"),
    ] {
        assert_eq!(default_route(explicit, model, configured), expected);
    }
    for kind in [
        Kind::OpenAi,
        Kind::Anthropic,
        Kind::OpenRouter,
        Kind::Gemini,
        Kind::Groq,
        Kind::Xai,
        Kind::Local,
        Kind::Copilot,
    ] {
        assert_eq!(
            format!(" {} ", kind.name().to_uppercase())
                .parse::<Kind>()
                .unwrap(),
            kind
        );
    }
    assert!("multi".parse::<Kind>().is_err());
    for (kind, selected, expected) in [
        (Kind::OpenAi, "", DEFAULT_CHAT_MODEL),
        (Kind::OpenAi, " Small ", "gpt-5.6-luna"),
        (Kind::OpenAi, "medium", "gpt-5.6-terra"),
        (Kind::OpenAi, "large", "gpt-5.6-sol"),
        (Kind::Anthropic, "", "claude-sonnet-4-6"),
        (Kind::Anthropic, "small", "claude-haiku-4-5"),
        (Kind::Anthropic, "large", "claude-opus-4-6"),
        (
            Kind::OpenRouter,
            "anthropic/claude-sonnet-5",
            "anthropic/claude-sonnet-5",
        ),
    ] {
        assert_eq!(kind.resolve_model(selected), expected);
    }
    let mut named = RouteSpec::new(Kind::Anthropic, AuthMode::AnthropicOAuth);
    named.prefix = Some(" Team-OAuth ".into());
    named.endpoint = Some("https://team.example.test/v1".into());
    named.account = Some("account-a".into());
    let canonical = RouteSpec::new(Kind::Anthropic, AuthMode::ApiKey)
        .scope()
        .unwrap();
    assert_eq!(named.scope().unwrap().route, "team-oauth");
    assert_eq!(canonical.endpoint, "https://api.anthropic.com");
    assert_eq!(canonical.account, None);
    assert_eq!(canonical.mode, AuthMode::ApiKey);
    named.prefix = Some(" ".into());
    assert_eq!(named.scope().unwrap().route, "anthropic");
    named.mode = AuthMode::OpenAiOAuth;
    assert!(named.scope().is_err());
    assert!(!supports_chat_completions(" GPT-5.3-CODEX "));
    assert!(supports_chat_completions("gpt-4.1"));
    for (model, protocol) in [
        ("CLAUDE-sonnet-5", Protocol::Anthropic),
        ("gpt-5.4", Protocol::Responses),
        ("codex-mini", Protocol::Responses),
        ("gpt-4.1", Protocol::Chat),
        ("gpt-6-astra", Protocol::Chat),
    ] {
        assert_eq!(adk_providers::copilot::protocol(model), protocol);
    }
}

#[tokio::test]
async fn complete_and_stream_share_named_override_default_and_nested_resolution() {
    let replaced = Arc::new(Probe::default());
    let selected = Arc::new(Probe::default());
    let mut routes = Routes::new(" Team ");
    routes
        .register_kind("openrouter", Kind::OpenRouter, replaced.clone())
        .unwrap();
    routes
        .register_kind(" OpenRouter ", Kind::OpenRouter, selected.clone())
        .unwrap();
    routes
        .register_kind("team", Kind::Anthropic, selected.clone())
        .unwrap();
    for (name, expected) in [
        ("small", "claude-haiku-4-5"),
        ("", "claude-sonnet-4-6"),
        ("team/", "claude-sonnet-4-6"),
        (
            "openrouter/anthropic/claude-sonnet-5",
            "anthropic/claude-sonnet-5",
        ),
    ] {
        let context = context();
        routes.complete(&context, request(name)).await.unwrap();
        let mut stream = routes.stream(&context, request(name)).await.unwrap();
        assert!(matches!(
            stream.next().await.unwrap(),
            Some(ModelEvent::Complete { .. })
        ));
        assert!(stream.next().await.unwrap().is_none());
        let calls = selected.requests.lock().unwrap();
        assert_eq!(&calls[calls.len() - 2..], &[expected, expected]);
    }
    assert!(replaced.requests.lock().unwrap().is_empty());
    assert!(routes.resolve("unknown/model").is_err());
    assert!(routes.resolve("OpenRouter/model").is_err());
    assert_eq!(
        routes.normalize_model_name("openrouter/anthropic/claude"),
        "anthropic/claude"
    );
    assert_eq!(
        routes.normalize_model_name("unknown/model"),
        "unknown/model"
    );
    assert!(routes.register("bad/prefix", selected).is_err());
}

#[test]
fn usage_and_cost_fixtures_cover_each_wire_semantics_and_cache_clamping() {
    for (protocol, wire_usage, context_tokens, expected) in [
        (
            Protocol::Responses,
            json!({"input_tokens":1000,"output_tokens":100,"input_tokens_details":{"cached_tokens":200,"cache_write_tokens":50}}),
            1000,
            0.00533,
        ),
        (
            Protocol::Chat,
            json!({"prompt_tokens":1000,"completion_tokens":100,"prompt_tokens_details":{"cached_tokens":200,"cache_write_tokens":50}}),
            1000,
            0.00533,
        ),
        (
            Protocol::Anthropic,
            json!({"input_tokens":1000,"output_tokens":100,"cache_read_input_tokens":200,"cache_creation_input_tokens":50}),
            1250,
            0.0047475,
        ),
    ] {
        let usage = wire::usage(&wire_usage, protocol);
        assert_eq!(usage.context_tokens, Some(context_tokens));
        let price = if protocol == Protocol::Anthropic {
            cost::anthropic("claude-sonnet-4-6", &usage)
        } else {
            cost::openai("gpt-5.6", &usage).unwrap()
        };
        near(price, expected);
    }
    for alias in [
        "gpt-5.6",
        "daybreak-blue-latest",
        "gpt-daybreak-blue-latest",
        "openai / gpt-5.6-sol",
    ] {
        near(cost::openai(alias, &usage()).unwrap(), 0.00533);
    }
    for (model, expected) in [
        ("gpt-4", 0.0025),
        ("gpt-4.1-mini", 0.0005),
        ("gpt-5.3-codex-spark", 0.00056),
        ("daybreak-red-latest", 0.01790625),
    ] {
        near(cost::openai(model, &usage()).unwrap(), expected);
    }
    assert_eq!(cost::openai("gateway/unknown", &usage()), None);
    for (input, expected) in [(272000, 2.72), (272001, 5.44002)] {
        near(
            cost::openai(
                "gpt-6-astra",
                &Usage {
                    input_tokens: input,
                    ..Usage::default()
                },
            )
            .unwrap(),
            expected,
        );
    }
    for (read, write, expected) in [(2000, 2000, 0.0004), (200, 2000, 0.00408)] {
        near(
            cost::openai(
                "gpt-5.6",
                &Usage {
                    input_tokens: 1000,
                    cache_read_tokens: read,
                    cache_creation_tokens: write,
                    ..Usage::default()
                },
            )
            .unwrap(),
            expected,
        );
    }
    for (model, expected) in [
        ("anthropic/claude-fable-5.1-20260101", 0.015675),
        ("claude-fable-5", 0.015825),
        ("claude-opus-4-1", 0.0237375),
        ("claude-4-1-opus", 0.0237375),
        ("claude-5-sonnet", 0.003165),
        ("claude-3-5-haiku-20241022", 0.001266),
        ("unknown", 0.0047475),
    ] {
        near(cost::anthropic(model, &usage()), expected);
    }
}

#[test]
fn named_route_cost_uses_kind_not_prefix_and_copilot_protocol() {
    let model = Arc::new(Probe::default());
    let mut routes = Routes::new("team");
    routes
        .register_kind("team", Kind::Anthropic, model.clone())
        .unwrap();
    routes
        .register_kind("subscription", Kind::Copilot, model.clone())
        .unwrap();
    routes
        .register_kind("gateway", Kind::OpenRouter, model.clone())
        .unwrap();
    routes.register("opaque", model).unwrap();
    near(
        routes.estimate_cost("team/large", &usage()).unwrap(),
        0.0079125,
    );
    near(
        routes
            .estimate_cost("subscription/claude-sonnet-5", &usage())
            .unwrap(),
        0.003165,
    );
    near(
        routes
            .estimate_cost("subscription/gpt-5.6", &usage())
            .unwrap(),
        0.00533,
    );
    assert_eq!(
        routes.estimate_cost("gateway/anthropic/claude-sonnet-5", &usage()),
        None
    );
    assert_eq!(routes.estimate_cost("opaque/gpt-5.6", &usage()), None);
    assert_eq!(routes.estimate_cost("missing/gpt-5.6", &usage()), None);
}

#[test]
fn metadata_catalog_picker_lookup_and_compaction_match_baseline() {
    let models = parse_model_metadata(br#"{"models":[
        {"slug":" Z ","priority":2,"visibility":"list"},
        {"slug":"gpt-6-astra","priority":1,"context_window":272000,"max_context_window":1000000,"auto_compact_token_limit":null,"effective_context_window_percent":95,"display_name":" Astra ","description":" Catalog ","default_reasoning_level":" HIGH ","supported_reasoning_levels":[{"effort":" LOW "},{"effort":""},{"effort":"ultra"}],"upgrade":{"model":" next "}},
        {"slug":"gpt-6-astra","priority":9}, {"slug":"hidden","visibility":" HIDE "}, {"slug":"alpha"}, {"slug":""}
    ],"data":[{"id":"ignored"}]}"#).unwrap();
    assert_eq!(models.len(), 4);
    let by_id = model_metadata_by_id(&models);
    assert!(!by_id.contains_key("openai/gpt-6-astra"));
    let astra = &by_id["gpt-6-astra"];
    assert_eq!(astra.priority, Some(1));
    assert_eq!(astra.resolved_context_window(), Some(272000));
    assert_eq!(astra.compaction_defaults(), Some((244800, 136000)));
    assert_eq!(astra.supported_reasoning_levels, ["low", "ultra"]);
    assert_eq!(astra.default_reasoning_level, "high");
    assert_eq!(astra.upgrade_model.as_deref(), Some("next"));
    assert_eq!(astra.display_name, "Astra");
    assert_eq!(
        picker_model_metadata(&models)
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["gpt-6-astra", "Z", "alpha"]
    );
    let models = parse_model_metadata(br#"{"data":[{"id":" B ","capabilities":{"limits":{"max_context_window_tokens":128000,"max_prompt_tokens":96000,"max_output_tokens":32000}}},{"id":"a","capabilities":{"limits":{"max_prompt_tokens":200000}}},{"id":"A"}]}"#).unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].context_window, Some(200000));
    assert_eq!(models[1].context_window, Some(128000));
    assert_eq!(models[1].max_output_tokens, Some(32000));
    for (context_window, limit, expected) in [
        (Some(1000), Some(990), Some((900, 500))),
        (Some(372000), Some(180000), Some((180000, 90000))),
        (None, Some(1000), Some((1000, 500))),
        (None, None, None),
    ] {
        assert_eq!(
            ModelMetadata {
                context_window,
                auto_compact_token_limit: limit,
                ..Default::default()
            }
            .compaction_defaults(),
            expected
        );
    }
    for invalid in [
        b"{}".as_slice(),
        b"not-json",
        br#"{"data":[{"id":""}]}"#,
        br#"{"models":[{"slug":"x","context_window":"secret"}]}"#,
    ] {
        let error = parse_model_metadata(invalid).unwrap_err();
        assert!(!format!("{error:?}").contains("secret"));
    }
}

struct Store(Mutex<Material>);
impl CredentialStore for Store {
    fn load<'a>(&'a self, _: &'a Context, _: &'a Scope) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().clone()) })
    }
    fn replace<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        revision: u64,
        material: Material,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async move {
            let mut value = self.0.lock().unwrap();
            if value.revision != revision {
                return Ok(false);
            }
            *value = material;
            Ok(true)
        })
    }
}
struct Rotate;
impl Refresh for Rotate {
    fn refresh<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        mut material: Material,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async move {
            material.access_token = Secret::new("catalog-rotated");
            material.revision += 1;
            Ok(material)
        })
    }
}
fn session(endpoint: &str, mode: AuthMode) -> Session {
    Session::new(
        Scope::new("catalog", endpoint, None, mode).unwrap(),
        Arc::new(Store(Mutex::new(Material {
            access_token: Secret::new("catalog-initial"),
            refresh_token: Some(Secret::new("catalog-refresh")),
            id_token: None,
            email: None,
            account: Some("fixture-account".into()),
            expires_at: None,
            last_refresh: Some(std::time::SystemTime::now()),
            revision: 0,
        }))),
        Arc::new(Rotate),
    )
    .unwrap()
}

#[tokio::test]
async fn explicit_metadata_get_refreshes_once_and_normalizes_endpoint() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for (status, token, body) in [
            ("401 Unauthorized", "catalog-initial", "{}"),
            (
                "200 OK",
                "catalog-rotated",
                r#"{"data":[{"id":"gpt-5.6"}]}"#,
            ),
        ] {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut chunk = [0u8; 1024];
                let n = socket.read(&mut chunk).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(bytes).unwrap().to_lowercase();
            assert!(request.starts_with("get /v1/models http/1.1"));
            assert!(request.contains(&format!("authorization: bearer {token}")));
            write!(
                socket,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    let session = session(
        &format!("http://{address}/v1/responses"),
        AuthMode::OpenAiOAuth,
    );
    let models = fetch_model_metadata(&context(), &session).await.unwrap();
    assert_eq!(models[0].id, "gpt-5.6");
    server.join().unwrap();
    let session = self::session(
        "https://chatgpt.com/backend-api/codex/responses",
        AuthMode::OpenAiOAuth,
    );
    assert_eq!(
        model_metadata_endpoint(&session).unwrap().as_str(),
        "https://chatgpt.com/backend-api/codex/models?client_version=0.153.4"
    );
    let mut context = context();
    context.deadline = Some(std::time::Instant::now());
    assert_eq!(
        fetch_model_metadata(&context, &session)
            .await
            .unwrap_err()
            .info
            .category,
        ErrorCategory::DeadlineExceeded
    );
}

#[cfg(feature = "runtime")]
#[tokio::test]
async fn runner_fallback_charges_actual_named_binding_in_complete_and_stream_modes() {
    use adk_runtime::*;
    struct HostFixture;
    impl Host for HostFixture {
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
    #[derive(Default)]
    struct Costs(Mutex<Vec<f64>>);
    impl RunHooks for Costs {
        fn observe<'a>(
            &'a self,
            _: &'a Context,
            observation: Observation,
        ) -> BoxFuture<'a, Result<(), Error>> {
            Box::pin(async move {
                if let Observation::Usage { cost, .. } = observation {
                    self.0.lock().unwrap().push(cost);
                }
                Ok(())
            })
        }
    }
    for streaming in [false, true] {
        let primary = Arc::new(Probe {
            fail: true,
            ..Default::default()
        });
        let backup = Arc::new(Probe::default());
        let mut routes = Routes::new("primary");
        routes
            .register_kind("primary", Kind::OpenAi, primary.clone())
            .unwrap();
        routes
            .register_kind("backup", Kind::Anthropic, backup.clone())
            .unwrap();
        let routes = Arc::new(routes);
        let mut agent = AgentConfig::new(
            "fixture",
            ModelBinding::streaming("primary/gpt-5.6", routes.clone()),
        );
        agent.fallbacks.push(ModelBinding::streaming(
            "backup/claude-sonnet-5",
            routes.clone(),
        ));
        let observed = Arc::new(Costs::default());
        let config = RunnerConfig {
            cost_estimator: Some(Arc::new(adk_providers::runtime::BaselineCosts(routes))),
            hooks: Some(observed.clone()),
            ..Default::default()
        };
        let runner = Runner::new(agent, config).unwrap();
        let request = RunRequest {
            input_provenance: Vec::new(),
            input: vec![],
            policy: RunPolicy {
                max_turns: 2.try_into().unwrap(),
                tools: ToolPolicy::default(),
                tool_use: ToolUseBehavior::Continue,
            },
        };
        let outcome = if streaming {
            runner
                .stream(context(), request, Arc::new(HostFixture))
                .finish()
                .await
        } else {
            runner.run(context(), request, Arc::new(HostFixture)).await
        }
        .unwrap();
        assert_eq!(outcome.result.responses.len(), 1);
        assert_eq!(outcome.result.usage.input_tokens, 1000);
        assert_eq!(*primary.requests.lock().unwrap(), ["gpt-5.6"]);
        assert_eq!(*backup.requests.lock().unwrap(), ["claude-sonnet-5"]);
        near(*observed.0.lock().unwrap().last().unwrap(), 0.003165);
    }
}

#[tokio::test]
async fn registering_scoped_specs_is_lazy_and_never_reads_another_routes_store() {
    struct ScopedStore(Mutex<Vec<Scope>>);
    impl CredentialStore for ScopedStore {
        fn load<'a>(
            &'a self,
            _: &'a Context,
            scope: &'a Scope,
        ) -> BoxFuture<'a, Result<Material, Error>> {
            Box::pin(async move {
                self.0.lock().unwrap().push(scope.clone());
                Err(Error::new(
                    ErrorCategory::PermissionDenied,
                    "fixture unavailable",
                ))
            })
        }
        fn replace<'a>(
            &'a self,
            _: &'a Context,
            _: &'a Scope,
            _: u64,
            _: Material,
        ) -> BoxFuture<'a, Result<bool, Error>> {
            Box::pin(async { panic!("unexpected refresh") })
        }
    }
    let canonical = Arc::new(ScopedStore(Mutex::new(vec![])));
    let named = Arc::new(ScopedStore(Mutex::new(vec![])));
    let mut routes = Routes::new("anthropic");
    routes
        .register_spec(
            &RouteSpec::new(Kind::Anthropic, AuthMode::ApiKey),
            canonical.clone(),
            Arc::new(Rotate),
        )
        .unwrap();
    let mut spec = RouteSpec::new(Kind::Anthropic, AuthMode::AnthropicOAuth);
    spec.prefix = Some(" Team ".into());
    spec.account = Some("team-account".into());
    routes
        .register_spec(&spec, named.clone(), Arc::new(Rotate))
        .unwrap();
    assert!(canonical.0.lock().unwrap().is_empty());
    assert!(named.0.lock().unwrap().is_empty());
    assert!(
        routes
            .complete(&context(), request("team/large"))
            .await
            .is_err()
    );
    assert!(
        routes
            .stream(&context(), request("team/small"))
            .await
            .is_err()
    );
    assert!(canonical.0.lock().unwrap().is_empty());
    assert_eq!(*named.0.lock().unwrap(), vec![spec.scope().unwrap(); 2]);
}

#[cfg(feature = "runtime")]
#[test]
fn numeric_runner_adapter_does_not_claim_unknown_prices_are_known() {
    use adk_runtime::CostEstimator;
    let mut routes = Routes::new("openai");
    routes
        .register_kind("openai", Kind::OpenAi, Arc::new(Probe::default()))
        .unwrap();
    let costs = adk_providers::runtime::BaselineCosts(Arc::new(routes));
    assert_eq!(costs.0.estimate_cost("unknown", &usage()), None);
    assert_eq!(costs.cost("unknown", &usage()), 0.0);
    near(costs.cost("gpt-5.6", &usage()), 0.00533);
}
