use adk_core::{BoxFuture, Context, Error, Model, ModelEvent, ModelRequest, StreamingModel};
use adk_providers::{auth::*, client::Provider, wire::Protocol};
use adk_runtime::CancellationToken;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    time::Duration,
};

struct StaticStore;
impl CredentialStore for StaticStore {
    fn load<'a>(&'a self, _: &'a Context, _: &'a Scope) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async {
            Ok(Material {
                access_token: Secret::new("fixture-token"),
                refresh_token: None,
                id_token: None,
                email: None,
                account: None,
                expires_at: None,
                last_refresh: None,
                revision: 0,
            })
        })
    }
    fn replace<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        _: u64,
        _: Material,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async { panic!("static token must not refresh") })
    }
}
struct NoRefresh;
impl Refresh for NoRefresh {
    fn refresh<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        _: Material,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async { panic!("static token must not refresh") })
    }
}
fn context() -> Context {
    Context {
        run_id: "http-test".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: Some(std::time::Instant::now() + Duration::from_secs(5)),
    }
}
fn request() -> ModelRequest {
    ModelRequest {
        model: "test-model".into(),
        instructions: "test".into(),
        input: vec![],
        tools: vec![],
        output_schema: None,
        output_schema_name: "output".into(),
        output_schema_strict: true,
        settings: Default::default(),
    }
}
fn provider(endpoint: &str) -> Provider {
    let scope = Scope::new("fixture", endpoint, None, AuthMode::ApiKey).unwrap();
    Provider::new(
        "fixture",
        Protocol::Chat,
        Arc::new(Session::new(scope, Arc::new(StaticStore), Arc::new(NoRefresh)).unwrap()),
    )
    .unwrap()
}
fn read_request(socket: &mut TcpStream) -> String {
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buf = [0; 4096];
    loop {
        let n = socket.read(&mut buf).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buf[..n]);
        if let Some(offset) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..offset]).to_lowercase();
            let len: usize = headers
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))
                .unwrap()
                .parse()
                .unwrap();
            if bytes.len() >= offset + 4 + len {
                break;
            }
        }
    }
    String::from_utf8(bytes).unwrap()
}
#[test]
fn named_routes_preserve_gateway_model_ids_and_reject_unknown_prefixes() {
    let mut routes = adk_providers::routing::Routes::new("default");
    let first = Arc::new(provider("http://127.0.0.1:1"));
    routes.register("default", first.clone()).unwrap();
    routes.register("openrouter", first).unwrap();
    assert_eq!(routes.resolve("plain-model").unwrap().1, "plain-model");
    assert_eq!(
        routes.resolve("openrouter/anthropic/claude").unwrap().1,
        "anthropic/claude"
    );
    assert!(routes.resolve("anthropic/claude").is_err());
    assert!(routes.resolve("openrouter/").is_err());
    assert_eq!(
        routes.normalize_model_name("openrouter/anthropic/claude"),
        "anthropic/claude"
    );
    assert_eq!(
        routes.normalize_model_name("unknown/model"),
        "unknown/model"
    );
    let replacement: Arc<dyn StreamingModel> = Arc::new(provider("http://127.0.0.1:2"));
    routes.register("openrouter", replacement.clone()).unwrap();
    assert!(Arc::ptr_eq(
        &routes.resolve("openrouter/model").unwrap().0,
        &replacement
    ));
}

#[tokio::test]
async fn complete_sends_captured_request_and_normalizes_usage() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let request = read_request(&mut socket);
        let body = r#"{"id":"c","choices":[{"message":{"content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":4}}}"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
        request
    });
    let provider = provider(&format!("http://{addr}/v1"));
    let mut input = request();
    input
        .settings
        .insert("prompt_cache_key".into(), serde_json::json!("host-prompt"));
    let ctx = context();
    let response = provider.complete(&ctx, input).await.unwrap();
    assert_eq!(response.usage.context_tokens, Some(12));
    assert_eq!(response.usage.cache_read_tokens, 4);
    let raw = response.raw.as_ref().unwrap();
    assert_eq!(raw["id"], "c");
    assert_eq!(raw["choices"][0]["message"]["content"], "hello");
    assert_eq!(raw["usage"]["prompt_tokens_details"]["cached_tokens"], 4);
    assert!(response.metadata.is_empty());
    let request = server.join().unwrap();
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
    assert!(
        request
            .to_lowercase()
            .contains("authorization: bearer fixture-token")
    );
    let body: serde_json::Value =
        serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    let scope = Scope::new(
        "fixture",
        &format!("http://{addr}/v1"),
        None,
        AuthMode::ApiKey,
    )
    .unwrap();
    let material = StaticStore.load(&ctx, &scope).await.unwrap();
    assert_eq!(
        body,
        serde_json::json!({"model":"test-model","stream":false,"max_tokens":16384,"messages":[{"role":"system","content":"test"}],"prompt_cache_key":cache_scope(&scope, &material, "host-prompt")})
    );
}
#[tokio::test]
async fn redirects_are_not_followed_or_leaked_into_diagnostics() {
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let target_addr = target.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        read_request(&mut socket);
        write!(socket,"HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{target_addr}/secret-location\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    let error = provider(&format!("http://{addr}"))
        .complete(&context(), request())
        .await
        .unwrap_err();
    server.join().unwrap();
    assert!(!format!("{error:?}").contains("secret-location"));
    assert!(!format!("{error:?}").contains("fixture-token"));
    assert_eq!(
        target.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
#[tokio::test]
async fn chunked_stream_emits_incremental_text_and_exactly_one_complete() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        read_request(&mut socket);
        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").unwrap();
        for event in [
            r#"{"choices":[{"index":0,"delta":{"content":"hé🙂"}}]}"#,
            r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ] {
            for byte in format!("data: {event}\r\n\r\n").as_bytes() {
                socket
                    .write_all(&[b'1', b'\r', b'\n', *byte, b'\r', b'\n'])
                    .unwrap();
            }
            socket.flush().unwrap();
        }
        // The client may drop the body immediately after the protocol terminal.
        let _ = socket.write_all(b"0\r\n\r\n");
    });
    let provider = provider(&format!("http://{addr}"));
    let context = context();
    let mut stream = provider.stream(&context, request()).await.unwrap();
    assert_eq!(
        stream.next().await.unwrap(),
        Some(ModelEvent::TextDelta {
            delta: "hé🙂".into()
        })
    );
    let mut completes = 0;
    while let Some(event) = stream.next().await.unwrap() {
        if let ModelEvent::Complete { response } = event {
            let raw = response.raw.as_ref().unwrap();
            assert_eq!(raw["choices"][0]["message"]["content"], "hé🙂");
            assert_eq!(raw["choices"][0]["finish_reason"], "stop");
            completes += 1;
        }
    }
    assert_eq!(completes, 1);
    assert_eq!(stream.next().await.unwrap(), None);
    server.join().unwrap();
}
#[tokio::test]
async fn premature_eof_errors_once_then_closes_owned_stream() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        read_request(&mut socket);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    let provider = provider(&format!("http://{addr}"));
    let context = context();
    let mut stream = provider.stream(&context, request()).await.unwrap();
    assert!(stream.next().await.is_err());
    assert_eq!(stream.next().await.unwrap(), None);
    server.join().unwrap();
}

#[tokio::test]
async fn native_compaction_uses_dedicated_endpoint_and_replayable_output() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let captured = read_request(&mut socket);
        let body = r#"{"output":[{"type":"compaction","id":"c","encrypted_content":"opaque"}],"usage":{"input_tokens":12,"output_tokens":2}}"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
        captured
    });
    let scope = Scope::new(
        "fixture",
        &format!("http://{addr}/v1"),
        None,
        AuthMode::ApiKey,
    )
    .unwrap();
    let provider = Provider::new(
        "fixture",
        Protocol::Responses,
        Arc::new(Session::new(scope, Arc::new(StaticStore), Arc::new(NoRefresh)).unwrap()),
    )
    .unwrap();
    #[cfg(not(feature = "runtime"))]
    let (items, usage) = {
        let result = provider.compact(&context(), request()).await.unwrap();
        (result.items, result.usage)
    };
    #[cfg(feature = "runtime")]
    let (items, usage) = {
        use adk_runtime::Compactor;
        struct Price;
        impl adk_runtime::CostEstimator for Price {
            fn cost(&self, _: &str, _: &adk_core::Usage) -> f64 {
                0.25
            }
        }
        let provider = Arc::new(provider);
        let mut routes = adk_providers::routing::Routes::new("fixture");
        routes.register("fixture", provider.clone()).unwrap();
        let compactor = adk_providers::runtime::NativeCompactor {
            routes: Arc::new(routes),
            provider,
            template: request(),
            costs: Arc::new(Price),
        };
        let result = compactor
            .compact(
                &context(),
                adk_runtime::CompactionRequest {
                    agent: "fixture".into(),
                    model: "test-model".into(),
                    history: vec![],
                    context_tokens: 20,
                    target_tokens: 5,
                },
            )
            .await
            .unwrap();
        assert_eq!(result.cost, 0.25);
        assert_eq!(
            result.context_tokens,
            adk_runtime::compaction::estimate_history_tokens(&result.history)
        );
        (result.history, result.usage)
    };
    assert!(
        matches!(&items[0], adk_core::RunItem::Compaction { compaction } if compaction.encrypted_content == "opaque")
    );
    assert_eq!(usage.input_tokens, 12);
    let captured = server.join().unwrap();
    assert!(captured.starts_with("POST /v1/responses/compact HTTP/1.1"));
    let body: serde_json::Value =
        serde_json::from_str(captured.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        body,
        serde_json::json!({"model":"test-model","input":[],"instructions":"test"})
    );
}

#[tokio::test]
#[cfg(feature = "runtime")]
async fn native_compactor_resolves_bindings_and_keeps_original_cost_names() {
    use adk_providers::{factory::Kind, routing::Routes, runtime::NativeCompactor};
    use adk_runtime::{CompactionRequest, Compactor, CostEstimator};
    use serde_json::json;
    use std::sync::Mutex;

    let cases = [
        ("work/gpt-5.4", "gpt-5.4"),
        ("work/small", "gpt-5.6-luna"),
        ("work/medium", "gpt-5.6-terra"),
        ("work/large", "gpt-5.6-sol"),
        ("small", "gpt-5.6-luna"),
        ("work/vendor/model", "vendor/model"),
    ];
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for (_, expected) in cases {
            let (mut socket, _) = listener.accept().unwrap();
            let captured = read_request(&mut socket);
            assert!(captured.starts_with("POST /v1/responses/compact HTTP/1.1"));
            let body: serde_json::Value =
                serde_json::from_str(captured.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(body["model"], expected);
            assert_eq!(
                body["input"],
                json!([{"role":"user","content":"runner history"}])
            );
            let body = r#"{"output":[{"type":"compaction","id":"c","encrypted_content":"opaque"}],"usage":{"input_tokens":12,"output_tokens":2}}"#;
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    let scope = Scope::new("work", &format!("http://{addr}/v1"), None, AuthMode::ApiKey).unwrap();
    let provider = Arc::new(
        Provider::new(
            "work",
            Protocol::Responses,
            Arc::new(Session::new(scope, Arc::new(StaticStore), Arc::new(NoRefresh)).unwrap()),
        )
        .unwrap(),
    );
    let mut routes = Routes::new("work");
    routes
        .register_kind("work", Kind::OpenAi, provider.clone())
        .unwrap();
    #[derive(Default)]
    struct Price(Mutex<Vec<String>>);
    impl CostEstimator for Price {
        fn cost(&self, model: &str, usage: &adk_core::Usage) -> f64 {
            assert_eq!(usage.input_tokens, 12);
            assert_eq!(usage.output_tokens, 2);
            self.0.lock().unwrap().push(model.into());
            0.25
        }
    }
    let costs = Arc::new(Price::default());
    let compactor = NativeCompactor {
        routes: Arc::new(routes),
        provider,
        template: request(),
        costs: costs.clone(),
    };
    for (binding, _) in cases {
        let result = compactor
            .compact(
                &context(),
                CompactionRequest {
                    agent: "fixture".into(),
                    model: binding.into(),
                    history: vec![adk_core::RunItem::Message {
                        message: adk_core::Message {
                            role: adk_core::Role::User,
                            content: vec![adk_core::Content::Text {
                                text: "runner history".into(),
                            }],
                        },
                    }],
                    context_tokens: 20,
                    target_tokens: 5,
                },
            )
            .await
            .unwrap();
        assert_eq!(result.cost, 0.25);
        assert!(
            matches!(&result.history[0], adk_core::RunItem::Compaction { compaction } if compaction.encrypted_content == "opaque")
        );
    }
    server.join().unwrap();
    assert_eq!(
        *costs.0.lock().unwrap(),
        cases.map(|(binding, _)| binding.to_owned())
    );
}

#[tokio::test]
#[cfg(feature = "runtime")]
async fn native_compactor_rejects_incompatible_fallbacks_before_http() {
    use adk_providers::{factory::Kind, routing::Routes, runtime::NativeCompactor};
    use adk_runtime::{CompactionRequest, Compactor, CostEstimator};

    struct NoCost;
    impl CostEstimator for NoCost {
        fn cost(&self, _: &str, _: &adk_core::Usage) -> f64 {
            panic!("rejected compaction must not be charged")
        }
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let scope = Scope::new("work", &format!("http://{addr}/v1"), None, AuthMode::ApiKey).unwrap();
    let session =
        Arc::new(Session::new(scope, Arc::new(StaticStore), Arc::new(NoRefresh)).unwrap());
    let provider = Arc::new(Provider::new("work", Protocol::Responses, session.clone()).unwrap());
    let other = Arc::new(Provider::new("work", Protocol::Responses, session.clone()).unwrap());
    let chat = Arc::new(Provider::new("chat", Protocol::Chat, session.clone()).unwrap());
    let anthropic = Arc::new(Provider::new("anthropic", Protocol::Anthropic, session).unwrap());
    let mut routes = Routes::new("work");
    routes
        .register_kind("work", Kind::OpenAi, provider.clone())
        .unwrap();
    routes
        .register_kind("fallback", Kind::OpenAi, other)
        .unwrap();
    routes
        .register_kind("chat", Kind::OpenRouter, chat.clone())
        .unwrap();
    routes
        .register_kind("anthropic", Kind::Anthropic, anthropic)
        .unwrap();
    let routes = Arc::new(routes);
    let mut compactor = NativeCompactor {
        routes,
        provider,
        template: request(),
        costs: Arc::new(NoCost),
    };
    for binding in [
        "fallback/small",
        "anthropic/small",
        "chat/small",
        "missing/small",
    ] {
        let error = compactor
            .compact(
                &context(),
                CompactionRequest {
                    agent: "fixture".into(),
                    model: binding.into(),
                    history: vec![],
                    context_tokens: 20,
                    target_tokens: 5,
                },
            )
            .await
            .err()
            .expect("incompatible fallback must fail");
        assert_eq!(
            error.info.category,
            if binding.starts_with("missing/") {
                adk_core::ErrorCategory::InvalidInput
            } else {
                adk_core::ErrorCategory::Unsupported
            }
        );
    }
    compactor.provider = chat;
    let error = compactor
        .compact(
            &context(),
            CompactionRequest {
                agent: "fixture".into(),
                model: "chat/small".into(),
                history: vec![],
                context_tokens: 20,
                target_tokens: 5,
            },
        )
        .await
        .err()
        .expect("Chat cannot compact even on the matching route");
    assert_eq!(error.info.category, adk_core::ErrorCategory::Unsupported);
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn copilot_factory_routes_models_and_preserves_wire_identity() {
    use adk_providers::factory::{Kind, RouteSpec};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut captured = Vec::new();
        for body in [
            r#"{"content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn"}"#,
            r#"{"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"status":"completed"}"#,
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
        ] {
            let (mut socket, _) = listener.accept().unwrap();
            captured.push(read_request(&mut socket));
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        }
        captured
    });
    let spec = RouteSpec {
        kind: Kind::Copilot,
        prefix: Some("work".into()),
        endpoint: Some(format!("http://{addr}/v1")),
        protocol: None,
        mode: AuthMode::CopilotOAuth,
        account: None,
    };
    let model = spec
        .build(Arc::new(StaticStore), Arc::new(NoRefresh))
        .unwrap();
    for name in ["copilot/claude-opus-4.6", "gpt-5.4", "gpt-4o"] {
        let mut input = request();
        input.model = name.into();
        model.complete(&context(), input).await.unwrap();
    }
    let captured = server.join().unwrap();
    for (i, path) in ["/v1/messages", "/v1/responses", "/v1/chat/completions"]
        .iter()
        .enumerate()
    {
        assert!(captured[i].starts_with(&format!("POST {path} HTTP/1.1")));
        let body: serde_json::Value =
            serde_json::from_str(captured[i].split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["stream"], i == 2);
        assert!(
            captured[i]
                .to_lowercase()
                .contains("copilot-integration-id: vscode-chat")
        );
        assert!(!captured[i].to_lowercase().contains("x-api-key:"));
    }
    let body: serde_json::Value =
        serde_json::from_str(captured[0].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["model"], "claude-opus-4.6");
    assert_eq!(body["max_tokens"], 64000);
}

#[tokio::test]
async fn copilot_chat_streams_buffered_tool_calls_and_shapes_reasoning() {
    use adk_core::{Reasoning, RunItem};
    use adk_providers::{
        factory::{Kind, RouteSpec},
        wire::encode_reasoning_details,
    };
    use serde_json::json;

    for buffered in [true, false] {
        for effort in [Some(" high "), None, Some("  ")] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let captured = read_request(&mut socket);
                let chunks = [
                    json!({"id":"chat-1","choices":[{"delta":{"reasoning_text":"Let me "}}]}),
                    json!({"choices":[{"delta":{"reasoning_text":"think","reasoning_opaque":"signature","content":"Checking."}}]}),
                    json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1","function":{"name":"Web","arguments":"{\"url\":"}}]}}]}),
                    json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"Fetch","arguments":"\"https://example.com\"}"}}]},"finish_reason":"tool_calls"}]}),
                    json!({"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":5,"prompt_tokens_details":{"cached_tokens":4}}}),
                ];
                let mut body: String = chunks
                    .iter()
                    .map(|chunk| format!("data: {chunk}\n\n"))
                    .collect();
                body.push_str("data: [DONE]\n\n");
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                captured
            });
            let spec = RouteSpec {
                kind: Kind::Copilot,
                prefix: Some("work".into()),
                endpoint: Some(format!("http://{addr}")),
                protocol: Some(Protocol::Chat),
                mode: AuthMode::CopilotOAuth,
                account: None,
            };
            let model = spec
                .build(Arc::new(StaticStore), Arc::new(NoRefresh))
                .unwrap();
            let mut input = request();
            input.model = "claude-opus-4.8".into();
            input.settings.insert("thinking_budget".into(), json!(2048));
            if let Some(effort) = effort {
                input
                    .settings
                    .insert("reasoning_effort".into(), json!(effort));
                input
                    .settings
                    .insert("reasoning".into(), json!({"effort":"low"}));
            }
            input.input = vec![
                RunItem::Reasoning {
                    reasoning: Reasoning {
                        text: "plaintext".into(),
                        ..Default::default()
                    },
                },
                RunItem::Reasoning {
                    reasoning: Reasoning {
                        signature: encode_reasoning_details(&json!([{"text":"structured"}])),
                        ..Default::default()
                    },
                },
                RunItem::Reasoning {
                    reasoning: Reasoning {
                        text: "copilot text".into(),
                        signature: "opaque".into(),
                        ..Default::default()
                    },
                },
            ];
            input.tools.push(adk_core::ToolDefinition {
                name: "WebFetch".into(),
                description: "fetch a url".into(),
                input_schema: serde_json::from_value(
                    json!({"type":"object","properties":{"url":{"type":"string"}}}),
                )
                .unwrap(),
                read_only: true,
                requires_approval: false,
            });
            let ctx = context();
            let response = if buffered {
                model.complete(&ctx, input).await.unwrap()
            } else {
                let mut stream = model.stream(&ctx, input).await.unwrap();
                let mut response = None;
                while let Some(event) = stream.next().await.unwrap() {
                    if let ModelEvent::Complete { response: complete } = event {
                        response = Some(complete);
                    }
                }
                response.unwrap()
            };
            assert_eq!(response.end_turn, Some(false));
            assert_eq!(response.response_id.as_deref(), Some("chat-1"));
            assert_eq!(response.usage.input_tokens, 12);
            assert_eq!(response.usage.output_tokens, 5);
            assert_eq!(response.usage.cache_read_tokens, 4);
            assert_eq!(response.usage.context_tokens, Some(12));
            assert!(response.items.iter().any(|item| matches!(item, RunItem::ToolCall { call }
                if call.id == "call-1" && call.name == "WebFetch" && call.arguments == json!({"url":"https://example.com"}))));
            assert!(
                response
                    .items
                    .iter()
                    .any(|item| matches!(item, RunItem::Reasoning { reasoning }
                if reasoning.text == "Let me think" && reasoning.signature == "signature"))
            );
            let captured = server.join().unwrap();
            assert!(captured.starts_with("POST /chat/completions HTTP/1.1"));
            let body: serde_json::Value =
                serde_json::from_str(captured.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(body["stream"], true);
            assert_eq!(body["stream_options"]["include_usage"], true);
            assert_eq!(
                body.get("reasoning_effort"),
                effort
                    .filter(|e| !e.trim().is_empty())
                    .map(|e| json!(e.trim()))
                    .as_ref()
            );
            assert!(body.get("reasoning").is_none());
            assert!(body.get("thinking_budget").is_none());
            let messages = body["messages"].as_array().unwrap();
            for message in messages {
                for field in ["reasoning", "reasoning_content", "reasoning_details"] {
                    assert!(message.get(field).is_none(), "{body}");
                }
            }
            assert!(
                messages
                    .iter()
                    .any(|message| message["reasoning_text"] == "copilot text"
                        && message["reasoning_opaque"] == "opaque")
            );
        }
    }
}

#[tokio::test]
async fn copilot_chat_effort_healing_preserves_payload_and_stops_at_terminal_effort() {
    use adk_core::{Content, Message, Reasoning, Role, RunItem};
    use adk_providers::factory::{Kind, RouteSpec};
    use serde_json::{Value, json};

    for buffered in [true, false] {
        for efforts in [["max", "xhigh", "high"], ["none", "minimal", "low"]] {
            for terminal_failure in [false, true] {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                listener.set_nonblocking(true).unwrap();
                let addr = listener.local_addr().unwrap();
                let server = std::thread::spawn(move || {
                    let mut requests = Vec::new();
                    let deadline = std::time::Instant::now() + Duration::from_secs(5);
                    for index in 0..3 {
                        let mut socket = loop {
                            match listener.accept() {
                                Ok((socket, _)) => break socket,
                                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                    assert!(std::time::Instant::now() < deadline, "missing retry");
                                    std::thread::sleep(Duration::from_millis(5));
                                }
                                Err(error) => panic!("{error}"),
                            }
                        };
                        let captured = read_request(&mut socket);
                        assert!(captured.starts_with("POST /chat/completions HTTP/1.1"));
                        requests.push(
                            serde_json::from_str::<Value>(
                                captured.split_once("\r\n\r\n").unwrap().1,
                            )
                            .unwrap(),
                        );
                        let (status, body) = if index < 2 || terminal_failure {
                            (
                                400,
                                r#"{"error":{"message":"reasoning effort rejected secret-fixture"}}"#,
                            )
                        } else {
                            (
                                200,
                                "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
                            )
                        };
                        write!(socket, "HTTP/1.1 {status} fixture\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                    }
                    (requests, listener)
                });
                let model = RouteSpec {
                    kind: Kind::Copilot,
                    prefix: Some("work".into()),
                    endpoint: Some(format!("http://{addr}")),
                    protocol: Some(Protocol::Chat),
                    mode: AuthMode::CopilotOAuth,
                    account: None,
                }
                .build(Arc::new(StaticStore), Arc::new(NoRefresh))
                .unwrap();
                let mut input = request();
                input.model = "gpt-4o".into();
                input
                    .settings
                    .insert("reasoning_effort".into(), json!(efforts[0]));
                input
                    .settings
                    .insert("reasoning".into(), json!({"effort":"low"}));
                input.input = vec![
                    RunItem::Message {
                        message: Message {
                            role: Role::User,
                            content: vec![Content::Text {
                                text: "original history".into(),
                            }],
                        },
                    },
                    RunItem::Reasoning {
                        reasoning: Reasoning {
                            text: "copilot text".into(),
                            signature: "opaque".into(),
                            ..Default::default()
                        },
                    },
                ];
                input.tools.push(adk_core::ToolDefinition {
                    name: "WebFetch".into(),
                    description: "fetch a url".into(),
                    input_schema: serde_json::from_value(
                        json!({"type":"object","properties":{"url":{"type":"string"}}}),
                    )
                    .unwrap(),
                    read_only: true,
                    requires_approval: false,
                });
                let ctx = context();
                let result = if buffered {
                    model.complete(&ctx, input).await
                } else {
                    match model.stream(&ctx, input).await {
                        Ok(mut stream) => {
                            let mut response = None;
                            while let Some(event) = stream.next().await.unwrap() {
                                if let ModelEvent::Complete { response: complete } = event {
                                    assert!(response.replace(complete).is_none());
                                }
                            }
                            Ok(response.unwrap())
                        }
                        Err(error) => Err(error),
                    }
                };
                let (requests, listener) = server.join().unwrap();
                if terminal_failure {
                    let error = result.unwrap_err();
                    let advice = model.retry_advice(&error).unwrap();
                    assert_eq!(advice.reason, "400");
                    assert!(!advice.should_retry);
                    assert!(!format!("{error:?}").contains("secret-fixture"));
                } else {
                    assert_eq!(result.unwrap().end_turn, Some(true));
                }
                assert_eq!(
                    listener.accept().unwrap_err().kind(),
                    std::io::ErrorKind::WouldBlock
                );
                assert_eq!(requests.len(), 3);
                let mut original = requests[0].clone();
                original.as_object_mut().unwrap().remove("reasoning_effort");
                assert_eq!(original["model"], "gpt-4o");
                assert_eq!(original["stream"], true);
                assert_eq!(original["stream_options"]["include_usage"], true);
                assert_eq!(original["tools"][0]["function"]["name"], "WebFetch");
                let messages = original["messages"].as_array().unwrap();
                assert!(messages.iter().any(|message| message["role"] == "user"
                    && message["content"].to_string().contains("original history")));
                assert!(
                    messages
                        .iter()
                        .any(|message| message["reasoning_text"] == "copilot text"
                            && message["reasoning_opaque"] == "opaque")
                );
                for (mut body, effort) in requests.into_iter().zip(efforts) {
                    assert_eq!(
                        body.as_object_mut().unwrap().remove("reasoning_effort"),
                        Some(json!(effort))
                    );
                    assert!(body.get("reasoning").is_none());
                    for message in body["messages"].as_array().unwrap() {
                        for field in ["reasoning", "reasoning_content", "reasoning_details"] {
                            assert!(message.get(field).is_none());
                        }
                    }
                    assert_eq!(body, original);
                }
            }
        }
    }
}

#[tokio::test]
async fn request_healing_is_bounded_and_preserves_original_payload() {
    for protocol in [Protocol::Chat, Protocol::Responses, Protocol::Anthropic] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for index in 0..3 {
                let (mut socket, _) = listener.accept().unwrap();
                let captured = read_request(&mut socket);
                requests.push(
                    serde_json::from_str::<serde_json::Value>(
                        captured.split_once("\r\n\r\n").unwrap().1,
                    )
                    .unwrap(),
                );
                let (status, body) = if index < 2 {
                    (
                        400,
                        if protocol == Protocol::Anthropic {
                            r#"{"error":{"message":"effort rejected secret-fixture"}}"#
                        } else {
                            r#"{"error":{"message":"reasoning effort rejected secret-fixture"}}"#
                        },
                    )
                } else {
                    (
                        200,
                        r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}],"output":[{"type":"message","content":[{"type":"output_text","text":"ok"}]}],"status":"completed"}"#,
                    )
                };
                write!(socket, "HTTP/1.1 {status} fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                if protocol == Protocol::Anthropic && index == 1 {
                    break;
                }
            }
            requests
        });
        let scope =
            Scope::new("fixture", &format!("http://{addr}"), None, AuthMode::ApiKey).unwrap();
        let model = Provider::new(
            "fixture",
            protocol,
            Arc::new(Session::new(scope, Arc::new(StaticStore), Arc::new(NoRefresh)).unwrap()),
        )
        .unwrap();
        let mut input = request();
        input.model = "claude-opus-4.7".into();
        input
            .settings
            .insert("reasoning_effort".into(), serde_json::json!("max"));
        let result = model.complete(&context(), input).await;
        if protocol == Protocol::Anthropic {
            let error = result.unwrap_err();
            assert!(!format!("{error:?}").contains("secret-fixture"));
        } else {
            result.unwrap();
        }
        let requests = server.join().unwrap();
        let efforts: Vec<_> = requests
            .iter()
            .map(|body| {
                if protocol == Protocol::Anthropic {
                    body["output_config"]["effort"].as_str().unwrap()
                } else {
                    body["reasoning"]["effort"].as_str().unwrap()
                }
            })
            .collect();
        assert_eq!(
            efforts,
            if protocol == Protocol::Anthropic {
                vec!["max", "high"]
            } else {
                vec!["max", "xhigh", "high"]
            }
        );
        for body in &requests {
            assert_eq!(body["model"], "claude-opus-4.7");
        }
    }
}

#[cfg(feature = "runtime")]
#[tokio::test]
async fn runner_consumes_http_phase_and_false_end_turn_before_final_answer() {
    use adk_core::*;
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
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut captured = Vec::new();
        for (phase, end_turn, text) in [
            ("commentary", false, "working"),
            ("final_answer", true, "done"),
        ] {
            let (mut socket, _) = listener.accept().unwrap();
            captured.push(read_request(&mut socket));
            let body = serde_json::json!({"status":"completed","end_turn":end_turn,"output":[{"type":"message","role":"assistant","phase":phase,"content":[{"type":"output_text","text":text}]}]}).to_string();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        captured
    });
    let scope = Scope::new("fixture", &format!("http://{addr}"), None, AuthMode::ApiKey).unwrap();
    let model = Arc::new(
        Provider::new(
            "fixture",
            Protocol::Responses,
            Arc::new(Session::new(scope, Arc::new(StaticStore), Arc::new(NoRefresh)).unwrap()),
        )
        .unwrap(),
    );
    let runner = Runner::new(
        AgentConfig::new("fixture", ModelBinding::streaming("gpt-5.6", model)),
        RunnerConfig::default(),
    )
    .unwrap();
    let result = runner
        .run(
            context(),
            RunRequest {
                input: vec![],
                policy: RunPolicy {
                    max_turns: 2.try_into().unwrap(),
                    tools: ToolPolicy::default(),
                    tool_use: ToolUseBehavior::Continue,
                },
            },
            Arc::new(HostFixture),
        )
        .await
        .unwrap();
    assert_eq!(result.result.responses.len(), 2);
    let captured = server.join().unwrap();
    let second: serde_json::Value =
        serde_json::from_str(captured[1].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(second["input"][0]["phase"], "commentary");
    assert_eq!(second["input"][0]["content"], "working");
}

#[tokio::test]
async fn anthropic_thinking_repair_is_learned_only_for_its_model() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..4 {
            let (mut socket, _) = listener.accept().unwrap();
            let captured = read_request(&mut socket);
            requests.push(
                serde_json::from_str::<serde_json::Value>(
                    captured.split_once("\r\n\r\n").unwrap().1,
                )
                .unwrap(),
            );
            let (status, body) = if index == 0 {
                (
                    400,
                    r#"{"error":{"message":"thinking.type.enabled unsupported"}}"#,
                )
            } else {
                (
                    200,
                    r#"{"content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn"}"#,
                )
            };
            write!(socket, "HTTP/1.1 {status} fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    let scope = Scope::new("fixture", &format!("http://{addr}"), None, AuthMode::ApiKey).unwrap();
    let model = Provider::new(
        "fixture",
        Protocol::Anthropic,
        Arc::new(Session::new(scope, Arc::new(StaticStore), Arc::new(NoRefresh)).unwrap()),
    )
    .unwrap();
    for name in ["claude-sonnet-4.5", "claude-sonnet-4.5", "claude-haiku-4.5"] {
        let mut input = request();
        input.model = name.into();
        input
            .settings
            .insert("reasoning_effort".into(), serde_json::json!("high"));
        model.complete(&context(), input).await.unwrap();
    }
    let requests = server.join().unwrap();
    let kinds: Vec<_> = requests
        .iter()
        .map(|body| body["thinking"]["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["enabled", "adaptive", "adaptive", "enabled"]);
}

#[test]
fn generation_metadata_resolves_routes_aliases_and_cache_accounting_without_io() {
    use adk_providers::{
        factory::{Kind, RouteSpec},
        routing::Routes,
    };
    for (protocol, includes_cache) in [
        (Protocol::Responses, true),
        (Protocol::Chat, true),
        (Protocol::Anthropic, false),
    ] {
        let scope = Scope::new("wire", "http://127.0.0.1:9", None, AuthMode::ApiKey).unwrap();
        let model = Arc::new(
            Provider::new(
                "wire",
                protocol,
                Arc::new(Session::new(scope, Arc::new(StaticStore), Arc::new(NoRefresh)).unwrap()),
            )
            .unwrap(),
        );
        let mut routes = Routes::new("named");
        routes.register_kind("named", Kind::OpenAi, model).unwrap();
        let info = routes.info("named/gpt-5.6");
        assert_eq!(info.provider, "wire");
        assert_eq!(info.model, "gpt-5.6");
        assert_eq!(info.input_tokens_include_cache, Some(includes_cache));
    }
    let model = RouteSpec {
        kind: Kind::Copilot,
        prefix: Some("work".into()),
        endpoint: Some("http://127.0.0.1:9".into()),
        protocol: None,
        mode: AuthMode::CopilotOAuth,
        account: None,
    }
    .build(Arc::new(StaticStore), Arc::new(NoRefresh))
    .unwrap();
    assert_eq!(
        model.info("claude-sonnet-4.5").input_tokens_include_cache,
        Some(false)
    );
    assert_eq!(model.info("gpt-5.6").input_tokens_include_cache, Some(true));
}
