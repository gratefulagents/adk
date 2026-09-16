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
        serde_json::json!({"model":"test-model","stream":false,"messages":[{"role":"system","content":"test"}],"prompt_cache_key":cache_scope(&scope, &material, "host-prompt")})
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
        if matches!(event, ModelEvent::Complete { .. }) {
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
        let compactor = adk_providers::runtime::NativeCompactor {
            provider: Arc::new(provider),
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
async fn copilot_factory_routes_models_and_preserves_wire_identity() {
    use adk_providers::factory::{Kind, RouteSpec};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut captured = Vec::new();
        for body in [
            r#"{"content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn"}"#,
            r#"{"output":[],"status":"completed"}"#,
            r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}"#,
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
