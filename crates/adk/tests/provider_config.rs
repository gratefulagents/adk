#![cfg(feature = "builder")]
use adk::{
    builder::{Builder, Config, Features},
    core::*,
    providers::{auth::*, factory::config::Config as ProviderConfig},
    runtime::CancellationToken,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    time::Duration,
};

struct Credentials(Mutex<Vec<Scope>>);
impl CredentialStore for Credentials {
    fn load<'a>(
        &'a self,
        _: &'a Context,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(scope.clone());
            assert_eq!(scope.route, "gateway");
            Ok(Material {
                access_token: Secret::new("host-owned"),
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
        Box::pin(async { panic!("unexpected refresh") })
    }
}
impl Refresh for Credentials {
    fn refresh<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        _: Material,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async { panic!("unexpected refresh") })
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

#[tokio::test]
async fn declarative_registry_runs_through_facade_builder() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buf = [0; 4096];
        let (headers, body) = loop {
            let n = socket.read(&mut buf).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8(bytes[..end].to_vec())
                    .unwrap()
                    .to_ascii_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break (
                        headers,
                        serde_json::from_slice::<serde_json::Value>(
                            &bytes[end + 4..end + 4 + length],
                        )
                        .unwrap(),
                    );
                }
            }
        };
        assert!(headers.starts_with("post /api/v1/chat/completions "));
        assert!(headers.contains("authorization: bearer host-owned"));
        assert_eq!(body["model"], "anthropic/claude-sonnet-4.6");
        let response = r#"{"id":"gen-1","model":"anthropic/claude-sonnet-4.6","choices":[{"index":0,"message":{"role":"assistant","content":"facade-ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#;
        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
    });
    let config: ProviderConfig = serde_json::from_value(serde_json::json!({
        "provider":"multi","default_provider":"gateway",
        "routes":[{"prefix":"gateway","provider":"openrouter","base_url":endpoint}]
    }))
    .unwrap();
    let store = Arc::new(Credentials(Mutex::new(Vec::new())));
    let routes = config
        .resolve()
        .unwrap()
        .build(|_, _| Ok(store.clone()), store.clone())
        .unwrap();
    let context = Context {
        run_id: "facade-config".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: Some(std::time::Instant::now() + Duration::from_secs(5)),
    };
    let mut bundle = Builder::new(Config {
        model: "gateway/anthropic/claude-sonnet-4.6".into(),
        features: Some(Features::default()),
        ..Default::default()
    })
    .routes(routes)
    .build(&context)
    .await
    .unwrap();
    assert!(store.0.lock().unwrap().is_empty());
    let outcome = bundle
        .run(context, vec![], Arc::new(TestHost))
        .await
        .unwrap();
    assert!(outcome.result.responses[0].items.iter().any(|item| matches!(item, RunItem::Message { message } if message.content.iter().any(|c| matches!(c, Content::Text { text } if text == "facade-ok")))));
    assert_eq!(store.0.lock().unwrap().len(), 1);
    bundle.close().await.unwrap();
    server.join().unwrap();
}
