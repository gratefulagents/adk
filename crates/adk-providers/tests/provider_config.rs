use adk_core::{BoxFuture, Content, Context, Error, ErrorCategory, Model, ModelRequest, RunItem};
use adk_providers::{auth::*, factory::config::Config, wire::Protocol};
use adk_runtime::CancellationToken;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    time::Duration,
};

#[path = "provider_config/fallbacks.rs"]
mod fallbacks;

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../fixtures/provider-config/observations.json"
    ))
    .unwrap()
}

#[test]
fn pinned_configuration_observations() {
    let fixture = fixture();
    assert_eq!(
        fixture["provenance"]["commit"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    for case in fixture["cases"].as_array().unwrap() {
        let config: Config = serde_json::from_value(case["input"].clone()).unwrap();
        let resolved = config.resolve().unwrap();
        assert_eq!(resolved.default, case["default"]);
        let routes: Vec<_> = resolved.routes.iter().map(|route| {
            let scope = route.spec.scope().unwrap();
            json!({"prefix": scope.route, "kind": route.spec.kind.name(), "endpoint": scope.endpoint,
                "key": route.api_key.as_ref().map_or("", Secret::expose),
                "auth": if scope.mode == AuthMode::AnthropicOAuth { "oauth" } else { "api-key" },
                "protocol": match route.spec.protocol.unwrap() { Protocol::Chat => "chat-completions", Protocol::Responses => "responses", Protocol::Anthropic => "anthropic" }})
        }).collect();
        assert_eq!(json!(routes), case["routes"]);
    }
}

#[test]
fn unsupported_and_ambiguous_inputs_are_explicit_and_redacted() {
    for input in [
        json!({"provider":"copilot"}),
        json!({"provider":"bogus"}),
        json!({"routes":[{"prefix":"missing"}]}),
        json!({"routes":[{"provider":"multi"}]}),
        json!({"routes":[{"provider":"openai","prefix":"bad/route"}]}),
        json!({"provider":"openai","auth_mode":"oauth"}),
        json!({"provider":"groq","auth_mode":"oauth"}),
        json!({"provider":"anthropic","provider_auth_modes":{"openai":"oauth"}}),
        json!({"provider":"anthropic","provider_api_modes":{"groq":"typo"}}),
        json!({"provider":"multi","default_provider":"copilot"}),
        json!({"provider":"openai","api_mode":"typo"}),
        json!({"provider":"anthropic","api_mode":"responses"}),
        json!({"provider_api_keys":{"OpenAI":"one"," openai ":"two"}}),
        json!({"provider_api_keys":{"copilot":"unsupported"}}),
        json!({"api_key":"do-not-print-secret","routes":[{"provider":"anthropic"}]}),
        json!({"base_url":"http://external.test"}),
        json!({"base_url":"https://user:do-not-print-secret@example.test"}),
        json!({"base_url":"https://example.test?api_key=do-not-print-secret"}),
        json!({"base_url":"https://example.test/#do-not-print-secret"}),
    ] {
        let config: Config = serde_json::from_value(input).unwrap();
        let error = config.resolve().err().expect("must reject configuration");
        assert!(!format!("{error:?}").contains("do-not-print-secret"));
    }
    for field in [
        "openai_oauth_path",
        "openai_oauth_account_id",
        "openai_auth_session",
        "anthropic_oauth_path",
        "copilot_oauth_path",
    ] {
        assert!(serde_json::from_value::<Config>(json!({field:"unsupported"})).is_err());
        assert!(
            serde_json::from_value::<Config>(
                json!({"routes":[{"provider":"anthropic",field:"unsupported"}]})
            )
            .is_err()
        );
    }
    assert!(serde_json::from_value::<Config>(json!({"model_fallbacks":"not-an-array"})).is_err());
    assert!(
        serde_json::from_value::<Config>(
            json!({"routes":[{"provider":"openrouter", "model_fallbacks":["backup"]}]})
        )
        .is_err()
    );
    assert_eq!(
        format!("{:?}", Secret::new("do-not-print-secret")),
        "[REDACTED]"
    );
    for expected in fixture()["errors"].as_object().unwrap().values() {
        assert_eq!(expected, true);
    }
}

#[test]
fn named_prefix_is_not_a_provider_kind() {
    let config: Config = serde_json::from_value(json!({
        "provider":"multi","default_provider":"copilot",
        "routes":[{"prefix":"copilot","provider":"anthropic"}]
    }))
    .unwrap();
    let resolved = config.resolve().unwrap();
    let route = resolved
        .routes
        .iter()
        .find(|route| route.spec.prefix() == "copilot")
        .unwrap();
    assert_eq!(route.spec.kind, adk_providers::factory::Kind::Anthropic);
    assert!(route.api_key.is_none());
}

struct Store {
    scope: Scope,
    token: Option<Secret>,
    reads: Arc<Mutex<Vec<Scope>>>,
}
impl CredentialStore for Store {
    fn load<'a>(
        &'a self,
        _: &'a Context,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async move {
            assert_eq!(scope, &self.scope);
            self.reads.lock().unwrap().push(scope.clone());
            let token = self.token.clone().ok_or_else(|| {
                Error::new(
                    ErrorCategory::PermissionDenied,
                    "missing fixture credential",
                )
            })?;
            Ok(Material {
                access_token: token,
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
        Box::pin(async { panic!("no refresh") })
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
        Box::pin(async { panic!("no refresh") })
    }
}
fn context() -> Context {
    Context {
        run_id: "config".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: Some(std::time::Instant::now() + Duration::from_secs(5)),
    }
}
fn request(model: &str) -> ModelRequest {
    ModelRequest {
        model: model.into(),
        input_provenance: vec![],
        input: vec![],
        instructions: "test".into(),
        tools: vec![],
        output_schema: None,
        output_schema_name: "output".into(),
        output_schema_strict: true,
        settings: Default::default(),
    }
}

#[tokio::test]
async fn named_routes_match_pinned_wire_and_only_acquire_selected_scopes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut observations = Vec::new();
        for _ in 0..3 {
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
                    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
                    let length: usize = headers
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break (
                            headers,
                            serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + length])
                                .unwrap(),
                        );
                    }
                }
            };
            let header = |name: &str| {
                headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, v)| v.trim().to_owned())
                    .unwrap_or_default()
            };
            let path = headers
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let flavor = if path.contains("chat/completions") {
                "openrouter"
            } else if header("authorization").is_empty() {
                "key"
            } else {
                "bearer"
            };
            observations.push(json!({"flavor":flavor,"path":path,"model":body["model"],"authorization":header("authorization"),"api_key":header("x-api-key"),"oauth_beta":header("anthropic-beta").contains("oauth-2025-04-20"),"text":"ok"}));
            let response = if flavor == "openrouter" {
                r#"{"id":"gen-1","model":"anthropic/claude-sonnet-4.6","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#
            } else {
                r#"{"id":"msg_test","type":"message","role":"assistant","content":[{"type":"text","text":"ok"}],"model":"claude-sonnet-4-5","stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#
            };
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
        }
        observations
    });
    let config: Config = serde_json::from_value(json!({
        "provider":"anthropic","api_key":"parent-not-inherited","base_url":"https://parent.invalid",
        "routes":[
            {"provider":"anthropic","prefix":"anthropic","api_key":"replaced","base_url":"https://replaced.invalid"},
            {"provider":"anthropic","api_key":" fixture-key ","base_url":endpoint},
            {"provider":"anthropic","prefix":"bearer","auth_mode":"oauth","api_key":"fixture-bearer","base_url":endpoint},
            {"provider":"openrouter","prefix":"gateway","api_key":"fixture-openrouter","base_url":format!("{endpoint}/api/v1")},
            {"provider":"anthropic","prefix":"missing","base_url":endpoint}
        ]
    })).unwrap();
    let resolved = config.resolve().unwrap();
    let reads = Arc::new(Mutex::new(Vec::new()));
    let routes = resolved
        .build(
            |scope, token| {
                Ok(Arc::new(Store {
                    scope: scope.clone(),
                    token: token.cloned(),
                    reads: reads.clone(),
                }))
            },
            Arc::new(NoRefresh),
        )
        .unwrap();
    assert!(reads.lock().unwrap().is_empty());
    for (index, model) in [
        "claude-sonnet-4-5",
        "bearer/claude-sonnet-4-5",
        "gateway/anthropic/claude-sonnet-4.6",
    ]
    .iter()
    .enumerate()
    {
        let response = routes.complete(&context(), request(model)).await.unwrap();
        assert!(response.items.iter().any(|item| matches!(item, RunItem::Message { message } if message.content.iter().any(|c| matches!(c, Content::Text { text } if text == "ok")))));
        assert_eq!(reads.lock().unwrap().len(), index + 1);
    }
    assert_eq!(json!(server.join().unwrap()), fixture()["wire"]);
    assert_eq!(
        reads
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.route.as_str())
            .collect::<Vec<_>>(),
        ["anthropic", "bearer", "gateway"]
    );
    let error = routes
        .complete(&context(), request("missing/claude-sonnet-4-5"))
        .await
        .err()
        .unwrap();
    assert_eq!(error.info.category, ErrorCategory::PermissionDenied);
    assert_eq!(reads.lock().unwrap().last().unwrap().route, "missing");
    let before = reads.lock().unwrap().len();
    assert!(
        routes
            .complete(&context(), request("unknown/model"))
            .await
            .is_err()
    );
    assert_eq!(reads.lock().unwrap().len(), before);
}
