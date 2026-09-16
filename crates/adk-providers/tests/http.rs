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
    let response = provider.complete(&context(), request()).await.unwrap();
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
    assert_eq!(
        body,
        serde_json::json!({"model":"test-model","stream":false,"messages":[{"role":"system","content":"test"}]})
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
