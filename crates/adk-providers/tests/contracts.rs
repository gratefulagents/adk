use adk_core::{BoxFuture, Context, Error, ErrorCategory, ModelRequest};
use adk_providers::{
    auth::*,
    client::StreamState,
    error::*,
    sse::Decoder,
    wire::{self, Protocol},
};
use adk_runtime::CancellationToken;
use reqwest::header::HeaderMap;
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};
use tokio::sync::Mutex;

fn context() -> Context {
    Context {
        run_id: "test".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn material() -> Material {
    Material {
        access_token: Secret::new("fixture-access"),
        refresh_token: Some(Secret::new("fixture-refresh")),
        id_token: None,
        email: None,
        account: Some("account-a".into()),
        expires_at: Some(SystemTime::UNIX_EPOCH),
        last_refresh: Some(SystemTime::UNIX_EPOCH),
        revision: 1,
    }
}
fn scope(mode: AuthMode) -> Scope {
    Scope::new(
        "route-a",
        "https://api.example.test/v1",
        Some("account-a".into()),
        mode,
    )
    .unwrap()
}
fn request() -> ModelRequest {
    ModelRequest {
        model: "fixture-model".into(),
        instructions: "Be precise".into(),
        input: Vec::new(),
        tools: Vec::new(),
        output_schema: None,
        output_schema_name: "output".into(),
        output_schema_strict: true,
        settings: Default::default(),
    }
}

#[test]
fn sse_every_byte_boundary_including_unicode_crlf_and_bom() {
    let data =
        "\u{feff}: comment\r\nevent: delta\r\ndata: hé🙂\r\ndata: two\r\n\r\ndata: [DONE]\n\n";
    for split in 0..=data.len() {
        let mut decoder = Decoder::default();
        let mut events = decoder.feed(&data.as_bytes()[..split]).unwrap();
        events.extend(decoder.feed(&data.as_bytes()[split..]).unwrap());
        decoder.finish().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event, "delta");
        assert_eq!(events[0].data, "hé🙂\ntwo");
        assert_eq!(events[1].data, "[DONE]");
    }
    let mut decoder = Decoder::default();
    let mut events = Vec::new();
    for byte in data.as_bytes() {
        events.extend(decoder.feed(&[*byte]).unwrap());
    }
    assert_eq!(events.len(), 2);
}
#[test]
fn sse_bounds_invalid_utf8_and_truncation_fail_closed() {
    assert!(Decoder::new(4).feed(b"data: too long\n\n").is_err());
    assert!(Decoder::default().feed(b"data: \xff\n\n").is_err());
    let mut decoder = Decoder::default();
    decoder.feed(b"data: unfinished").unwrap();
    assert!(decoder.finish().is_err());
    assert!(decoder.feed(b"\n\n").is_err());
}
#[test]
fn retry_headers_match_reference_floor_precedence_dates_and_caps() {
    let mut headers = HeaderMap::new();
    for (raw, seconds) in [
        ("1", 1),
        ("999", 1),
        ("1999", 1),
        ("2999", 2),
        ("18446744073709551615", 300),
    ] {
        headers.insert("retry-after-ms", raw.parse().unwrap());
        assert_eq!(
            retry_after(&headers, SystemTime::UNIX_EPOCH),
            Duration::from_secs(seconds)
        );
    }
    headers.insert("retry-after-ms", "0".parse().unwrap());
    headers.insert("retry-after", "999999".parse().unwrap());
    assert_eq!(
        retry_after(&headers, SystemTime::UNIX_EPOCH),
        Duration::from_secs(300)
    );
    headers.insert(
        "retry-after",
        "Thu, 01 Jan 1970 00:01:00 GMT".parse().unwrap(),
    );
    assert_eq!(
        retry_after(&headers, SystemTime::UNIX_EPOCH),
        Duration::from_secs(60)
    );
    for status in [301, 400, 401, 403, 408, 409, 422] {
        assert!(!RequestFailure::http(status, &headers, SystemTime::now()).retryable());
    }
    for status in [429, 500, 502, 503, 529] {
        assert!(RequestFailure::http(status, &headers, SystemTime::now()).retryable());
    }
}
#[test]
fn scopes_reject_credential_urls_and_nonloopback_plaintext() {
    for endpoint in [
        "http://api.example.test",
        "https://user:password@example.test",
        "https://example.test/?token=secret",
        "https://example.test/#secret",
        "file:///secret",
    ] {
        assert!(Scope::new("test", endpoint, None, AuthMode::ApiKey).is_err());
    }
    for endpoint in [
        "https://example.test/v1/",
        "http://127.0.0.1:1234",
        "http://[::1]:1234",
    ] {
        assert!(Scope::new("test", endpoint, None, AuthMode::ApiKey).is_ok());
    }
}
#[test]
fn diagnostics_and_header_debug_are_secret_safe() {
    let material = material();
    let scope = scope(AuthMode::OpenAiOAuth);
    let headers = headers(&scope, &material, false).unwrap();
    let output = format!("{material:?} {headers:?}");
    assert!(!output.contains("fixture-access"));
    assert!(!output.contains("fixture-refresh"));
    assert!(headers["authorization"].is_sensitive());
    assert!(headers["chatgpt-account-id"].is_sensitive());
}
#[test]
fn cache_namespace_is_endpoint_route_mode_account_token_scoped() {
    let original = scope(AuthMode::ApiKey);
    let material = material();
    let initial = cache_scope(&original, &material, "prompt");
    for change in 0..5 {
        let mut scope = original.clone();
        let mut material = material.clone();
        match change {
            0 => scope.endpoint.push_str("/other"),
            1 => scope.route.push('b'),
            2 => scope.mode = AuthMode::OpenAiOAuth,
            3 => material.account = Some("account-b".into()),
            _ => material.access_token = Secret::new("rotated"),
        }
        assert_ne!(initial, cache_scope(&scope, &material, "prompt"));
    }
    assert_ne!(initial, cache_scope(&original, &material, "another prompt"));
}
#[test]
fn refresh_lead_adapts_to_short_lived_anthropic_tokens() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10000);
    let mut material = material();
    material.last_refresh = Some(now);
    material.expires_at = Some(now + Duration::from_secs(3600));
    assert!(!material.needs_refresh(AuthMode::AnthropicOAuth, now));
    assert!(material.needs_refresh(AuthMode::AnthropicOAuth, now + Duration::from_secs(1800)));
    material.expires_at = None;
    assert!(!material.needs_refresh(AuthMode::AnthropicOAuth, now + Duration::from_secs(999999)));
    assert!(material.needs_refresh(AuthMode::OpenAiOAuth, now + Duration::from_secs(8 * 86400)));
}
struct Store {
    value: Mutex<Material>,
    writes: AtomicUsize,
}
impl CredentialStore for Store {
    fn load<'a>(&'a self, _: &'a Context, _: &'a Scope) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async { Ok(self.value.lock().await.clone()) })
    }
    fn replace<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        revision: u64,
        mut material: Material,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async move {
            let mut value = self.value.lock().await;
            if value.revision != revision {
                return Ok(false);
            }
            material.revision += 1;
            *value = material;
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        })
    }
}
struct Refresher(AtomicUsize);
impl Refresh for Refresher {
    fn refresh<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        mut material: Material,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            material.access_token = Secret::new("new-access");
            material.expires_at = None;
            Ok(material)
        })
    }
}
#[tokio::test]
async fn refresh_singleflight_reloads_external_rotation_and_validates_account() {
    let store = Arc::new(Store {
        value: Mutex::new(material()),
        writes: AtomicUsize::new(0),
    });
    let refresh = Arc::new(Refresher(AtomicUsize::new(0)));
    let session = Arc::new(
        Session::new(
            scope(AuthMode::AnthropicOAuth),
            store.clone(),
            refresh.clone(),
        )
        .unwrap(),
    );
    let context = context();
    let (a, b, c) = tokio::join!(
        session.material(&context),
        session.material(&context),
        session.material(&context)
    );
    for value in [a, b, c] {
        assert_eq!(value.unwrap().access_token.expose(), "new-access");
    }
    assert_eq!(refresh.0.load(Ordering::SeqCst), 1);
    assert_eq!(store.writes.load(Ordering::SeqCst), 1);
    store.value.lock().await.access_token = Secret::new("external-rotation");
    assert_eq!(
        session
            .material(&context)
            .await
            .unwrap()
            .access_token
            .expose(),
        "external-rotation"
    );
    store.value.lock().await.account = Some("wrong-account".into());
    assert_eq!(
        session.material(&context).await.unwrap_err().info.category,
        ErrorCategory::PermissionDenied
    );
}
struct Pending;
impl Refresh for Pending {
    fn refresh<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        _: Material,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(std::future::pending())
    }
}
#[tokio::test]
async fn cancellation_interrupts_refresh_and_releases_the_scope_lock() {
    let store = Arc::new(Store {
        value: Mutex::new(material()),
        writes: AtomicUsize::new(0),
    });
    let session = Session::new(scope(AuthMode::AnthropicOAuth), store, Arc::new(Pending)).unwrap();
    let cancellation = Arc::new(CancellationToken::new());
    let mut context = context();
    context.cancellation = cancellation.clone();
    let operation = session.material(&context);
    let cancel = async {
        tokio::task::yield_now().await;
        cancellation.cancel();
    };
    let (result, ()) = tokio::join!(operation, cancel);
    assert_eq!(result.unwrap_err().info.category, ErrorCategory::Cancelled);
    let mut second = context.clone();
    second.cancellation = Arc::new(CancellationToken::new());
    second.deadline = Some(std::time::Instant::now());
    assert_eq!(
        session.material(&second).await.unwrap_err().info.category,
        ErrorCategory::DeadlineExceeded
    );
}
#[test]
fn cache_usage_uses_provider_specific_subset_semantics() {
    // Values and cache_write spelling from SDK internal/openai/cache_usage_test.go.
    let input = json!({"input_tokens":100,"output_tokens":20,"input_tokens_details":{"cached_tokens":70,"cache_write_tokens":11}});
    let usage = wire::usage(&input, Protocol::Responses);
    assert_eq!(
        (
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            usage.cache_creation_tokens,
            usage.context_tokens
        ),
        (100, 20, 70, 11, Some(100))
    );
    let usage = wire::usage(
        &json!({"input_tokens":19,"output_tokens":20,"cache_read_input_tokens":70,"cache_creation_input_tokens":11}),
        Protocol::Anthropic,
    );
    assert_eq!(usage.context_tokens, Some(100));
}
#[test]
fn structural_settings_cannot_overwrite_model_or_history() {
    let mut input = request();
    input.settings.insert("messages".into(), json!([]));
    assert!(wire::request(&input, Protocol::Chat, false).is_err());
}
#[test]
fn chat_stream_requires_finish_and_accumulates_reasoning_and_tool_arguments() {
    let mut state = StreamState::new(Protocol::Chat);
    assert!(state.event("[DONE]").is_err());
    state.event(r#"{"choices":[{"index":0,"delta":{"reasoning_content":"think","tool_calls":[{"index":0,"id":"call-1","function":{"name":"lookup","arguments":"{\"x\":"}}]}}]}"#).unwrap();
    state.event(r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]},"finish_reason":"tool_calls"}]}"#).unwrap();
    state
        .event(r#"{"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":2}}"#)
        .unwrap();
    let events = state.event("[DONE]").unwrap();
    let adk_core::ModelEvent::Complete { response } = events.last().unwrap() else {
        panic!()
    };
    assert_eq!(response.end_turn, Some(false));
    assert_eq!(response.usage.input_tokens, 10);
    let adk_core::RunItem::ToolCall { call } = &response.items[1] else {
        panic!()
    };
    assert_eq!(call.id, "call-1");
    assert_eq!(call.arguments, json!({"x":1}));
    assert!(state.event("[DONE]").is_err());
}
#[test]
fn anthropic_stream_merges_usage_and_preserves_signature() {
    let mut state = StreamState::new(Protocol::Anthropic);
    for event in [
        json!({"type":"message_start","message":{"id":"m","usage":{"input_tokens":10,"cache_read_input_tokens":50}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"think"}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signed"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}),
    ] {
        state.event(&event.to_string()).unwrap();
    }
    let events = state.event(r#"{"type":"message_stop"}"#).unwrap();
    let adk_core::ModelEvent::Complete { response } = events.last().unwrap() else {
        panic!()
    };
    assert_eq!(response.usage.context_tokens, Some(60));
    assert_eq!(response.usage.output_tokens, 2);
    assert!(format!("{:?}", response.items).contains("signed"));
}
#[test]
fn opaque_compaction_and_reasoning_survive_exact_codex_replay_shape() {
    let response = wire::response(&json!({"status":"completed","output":[
        {"type":"reasoning","id":"r","summary":[{"type":"summary_text","text":"summary"}],"encrypted_content":"reasoning-opaque"},
        {"type":"compaction","id":"c","encrypted_content":"compact-opaque"}]}),Protocol::Responses).unwrap();
    let mut input = request();
    input.input = response.items;
    let body = wire::request(&input, Protocol::Responses, false).unwrap();
    let golden: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/providers/continuation.json"
    ))
    .unwrap();
    assert_eq!(body["input"], golden["responses_input"]);
    assert_eq!(
        wire::decode_reasoning_details(golden["reasoning_details_signature"].as_str().unwrap())
            .unwrap(),
        golden["reasoning_details_json"]
    );
    assert!(wire::request(&input, Protocol::Chat, false).is_err());
}

#[test]
fn responses_cache_fixture_and_cost_aliases_preserve_subsets() {
    let body = serde_json::from_str(include_str!(
        "../../../fixtures/providers/responses-cache.json"
    ))
    .unwrap();
    let response = wire::response(&body, Protocol::Responses).unwrap();
    assert_eq!(response.usage.cache_creation_tokens, 11);
    let cost = adk_providers::cost::openai("gpt-5.6", &response.usage).unwrap();
    let expected = (19.0 * 4.0 + 70.0 * 0.4 + 11.0 * 5.0 + 20.0 * 20.0) / 1_000_000.0;
    assert!((cost - expected).abs() < 1e-12);
    assert_eq!(
        Some(cost),
        adk_providers::cost::openai("openai/daybreak-blue-latest", &response.usage)
    );
    assert_eq!(
        None,
        adk_providers::cost::openai("unknown", &response.usage)
    );
    let mut usage = response.usage.clone();
    usage.input_tokens = 272_001;
    assert!(adk_providers::cost::openai("gpt-5.6", &usage).unwrap() > 2.0);
    usage.input_tokens = 1;
    usage.cache_read_tokens = 1000;
    usage.cache_creation_tokens = 1000;
    assert!(adk_providers::cost::openai("gpt-5.6", &usage).unwrap() >= 0.0);
    assert_eq!(
        adk_providers::cost::anthropic("anthropic/claude-fable-5.1-20260101", &usage),
        adk_providers::cost::anthropic("claude-fable-5-1", &usage)
    );
}

#[test]
fn responses_tool_delta_uses_call_id_not_output_item_id() {
    let mut state = StreamState::new(Protocol::Responses);
    state.event(r#"{"type":"response.output_item.added","item":{"type":"function_call","id":"item-1","call_id":"call-1"}}"#).unwrap();
    let events = state
        .event(
            r#"{"type":"response.function_call_arguments.delta","item_id":"item-1","delta":"{}"}"#,
        )
        .unwrap();
    assert_eq!(
        events,
        vec![adk_core::ModelEvent::ToolArgumentsDelta {
            call_id: "call-1".into(),
            delta: "{}".into()
        }]
    );
}

struct FailedRefresh;
impl Refresh for FailedRefresh {
    fn refresh<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        _: Material,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async {
            Err(Error::new(
                ErrorCategory::Provider,
                "fixture refresh failure",
            ))
        })
    }
}
#[tokio::test]
async fn anthropic_refresh_grace_never_reuses_a_rejected_token() {
    let mut value = material();
    value.expires_at = Some(SystemTime::now() + Duration::from_secs(60));
    let store = Arc::new(Store {
        value: Mutex::new(value),
        writes: AtomicUsize::new(0),
    });
    let session = Session::new(
        scope(AuthMode::AnthropicOAuth),
        store.clone(),
        Arc::new(FailedRefresh),
    )
    .unwrap();
    let ctx = context();
    let original = session.material(&ctx).await.unwrap();
    session.reject(&ctx, &original.access_token).await.unwrap();
    session
        .reject(&ctx, &Secret::new("older-rejected-token"))
        .await
        .unwrap();
    assert!(session.material(&ctx).await.is_err());
    {
        let mut current = store.value.lock().await;
        current.access_token = Secret::new("host-rotation");
        current.expires_at = None;
    }
    assert_eq!(
        session.material(&ctx).await.unwrap().access_token.expose(),
        "host-rotation"
    );
}
#[tokio::test]
async fn stale_unauthorized_does_not_refresh_new_host_credential() {
    let mut value = material();
    value.expires_at = None;
    value.access_token = Secret::new("host-rotation");
    let store = Arc::new(Store {
        value: Mutex::new(value),
        writes: AtomicUsize::new(0),
    });
    let refresh = Arc::new(Refresher(AtomicUsize::new(0)));
    let session = Session::new(scope(AuthMode::AnthropicOAuth), store, refresh.clone()).unwrap();
    let ctx = context();
    session
        .reject(&ctx, &Secret::new("old-token"))
        .await
        .unwrap();
    assert_eq!(
        session.material(&ctx).await.unwrap().access_token.expose(),
        "host-rotation"
    );
    assert_eq!(refresh.0.load(Ordering::SeqCst), 0);
}
#[test]
fn signed_redacted_and_gateway_reasoning_roundtrip_in_order() {
    let mut req = request();
    let body = json!({"content":[{"type":"thinking","thinking":"thought","signature":"signed"},
        {"type":"redacted_thinking","data":"redacted"},{"type":"text","text":"answer"}],"stop_reason":"end_turn"});
    req.input = wire::response(&body, Protocol::Anthropic).unwrap().items;
    let replay = wire::request(&req, Protocol::Anthropic, false).unwrap();
    assert_eq!(replay["messages"][0]["content"], body["content"]);
    let details = json!([{"type":"reasoning.encrypted","data":"opaque","index":0}]);
    let body = json!({"choices":[{"message":{"reasoning_details":details,"content":"answer"},"finish_reason":"stop"}]});
    req.input = wire::response(&body, Protocol::Chat).unwrap().items;
    assert_eq!(
        wire::request(&req, Protocol::Chat, false).unwrap()["messages"][1]["reasoning_details"],
        details
    );
}

#[test]
fn canonical_factory_scopes_every_baseline_leg_without_credential_inheritance() {
    use adk_providers::factory::{Kind, RouteSpec};
    for (kind, mode, endpoint, protocol) in [
        (
            Kind::OpenAi,
            AuthMode::ApiKey,
            "https://api.openai.com/v1",
            Protocol::Responses,
        ),
        (
            Kind::OpenAi,
            AuthMode::OpenAiOAuth,
            "https://chatgpt.com/backend-api/codex",
            Protocol::Responses,
        ),
        (
            Kind::Anthropic,
            AuthMode::ApiKey,
            "https://api.anthropic.com",
            Protocol::Anthropic,
        ),
        (
            Kind::Anthropic,
            AuthMode::AnthropicOAuth,
            "https://api.anthropic.com",
            Protocol::Anthropic,
        ),
        (
            Kind::OpenRouter,
            AuthMode::ApiKey,
            "https://openrouter.ai/api/v1",
            Protocol::Chat,
        ),
        (
            Kind::Gemini,
            AuthMode::ApiKey,
            "https://generativelanguage.googleapis.com/v1beta/openai",
            Protocol::Chat,
        ),
        (
            Kind::Groq,
            AuthMode::ApiKey,
            "https://api.groq.com/openai/v1",
            Protocol::Chat,
        ),
        (
            Kind::Xai,
            AuthMode::ApiKey,
            "https://api.x.ai/v1",
            Protocol::Responses,
        ),
        (
            Kind::Local,
            AuthMode::Anonymous,
            "http://localhost:11434/v1",
            Protocol::Chat,
        ),
        (
            Kind::Local,
            AuthMode::ApiKey,
            "http://localhost:11434/v1",
            Protocol::Chat,
        ),
        (
            Kind::Copilot,
            AuthMode::CopilotOAuth,
            "https://api.individual.githubcopilot.com",
            Protocol::Chat,
        ),
    ] {
        let mut spec = RouteSpec {
            kind,
            prefix: None,
            endpoint: None,
            protocol: None,
            mode,
            account: None,
        };
        assert_eq!(spec.scope().unwrap().endpoint, endpoint);
        assert_eq!(kind.protocol(), protocol);
        let canonical = spec.scope().unwrap();
        spec.prefix = Some("independent".into());
        assert_ne!(spec.scope().unwrap(), canonical);
        let store = Arc::new(Store {
            value: Mutex::new(material()),
            writes: AtomicUsize::new(0),
        });
        spec.build(store, Arc::new(FailedRefresh)).unwrap();
    }
}
#[test]
fn attribution_and_subscription_headers_do_not_leak_to_unrelated_hosts() {
    let value = material();
    for (url, expected) in [
        ("https://openrouter.ai/api/v1", true),
        ("https://eu.openrouter.ai/api/v1", true),
        ("https://openrouter.ai.evil.test/v1", false),
        ("https://example.test/v1", false),
    ] {
        let scope = Scope::new("named", url, None, AuthMode::ApiKey).unwrap();
        assert_eq!(
            headers(&scope, &value, false)
                .unwrap()
                .contains_key("http-referer"),
            expected
        );
    }
    let h = headers(&scope(AuthMode::CopilotOAuth), &value, true).unwrap();
    assert_eq!(h["copilot-integration-id"], "vscode-chat");
    assert_eq!(h["anthropic-beta"], "interleaved-thinking-2025-05-14");
    assert!(!h.contains_key("x-api-key"));
    let h = headers(&scope(AuthMode::AnthropicOAuth), &value, true).unwrap();
    assert_eq!(h["anthropic-beta"], "oauth-2025-04-20");
    assert_eq!(h["user-agent"], "claude-cli/2.1.158 (external, cli)");
}

struct RacingRefresh(Arc<Store>);
impl Refresh for RacingRefresh {
    fn refresh<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        mut value: Material,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async move {
            let mut external = self.0.value.lock().await;
            external.access_token = Secret::new("external-winner");
            external.expires_at = None;
            external.revision += 1;
            value.access_token = Secret::new("discarded-refresh");
            value.expires_at = None;
            Ok(value)
        })
    }
}
#[tokio::test]
async fn refresh_cas_loser_never_returns_its_unpersisted_credential() {
    let store = Arc::new(Store {
        value: Mutex::new(material()),
        writes: AtomicUsize::new(0),
    });
    let session = Session::new(
        scope(AuthMode::AnthropicOAuth),
        store.clone(),
        Arc::new(RacingRefresh(store.clone())),
    )
    .unwrap();
    assert_eq!(
        session
            .material(&context())
            .await
            .unwrap()
            .access_token
            .expose(),
        "external-winner"
    );
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
}

#[test]
fn oauth_cache_affinity_survives_refresh_but_never_missing_identity() {
    let scope = scope(AuthMode::OpenAiOAuth);
    let mut material = material();
    let initial = cache_scope(&scope, &material, "prompt");
    material.access_token = Secret::new("rotated-access");
    assert_eq!(cache_scope(&scope, &material, " prompt "), initial);
    material.account = None;
    assert!(cache_scope(&scope, &material, "prompt").is_empty());
}

#[test]
fn request_settings_match_executed_go_chat_and_responses_goldens() {
    let golden: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/providers/continuation.json"
    ))
    .unwrap();
    let mut req = request();
    req.model = "gpt-5.6".into();
    req.input = vec![adk_core::RunItem::Message {
        message: adk_core::Message {
            role: adk_core::Role::User,
            content: vec![adk_core::Content::Text {
                text: "hello".into(),
            }],
        },
    }];
    req.settings
        .insert("reasoning_effort".into(), json!("high"));
    for (protocol, key) in [
        (Protocol::Responses, "responses_request"),
        (Protocol::Chat, "chat_request"),
    ] {
        let mut actual = wire::request(&req, protocol, false).unwrap();
        // Transport sets stream separately; Rust additionally defaults store=false
        // to avoid server-side history retention. Compare every other wire field.
        actual.as_object_mut().unwrap().remove("stream");
        actual.as_object_mut().unwrap().remove("store");
        assert_eq!(actual, golden[key]);
    }
}
#[test]
fn responses_effort_none_budget_and_encrypted_include_are_preserved() {
    for (model, expected) in [
        ("gpt-5", "minimal"),
        ("gpt-5.1", "none"),
        ("gpt-5.6-codex", "minimal"),
        ("openai/gpt-6", "none"),
    ] {
        let mut req = request();
        req.model = model.into();
        req.settings
            .insert("reasoning_effort".into(), json!("none"));
        let body = wire::request(&req, Protocol::Responses, true).unwrap();
        assert_eq!(body["reasoning"]["effort"], expected);
        assert!(body["reasoning"].get("summary").is_none());
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
    }
    let mut req = request();
    req.settings.insert("thinking_budget".into(), json!(12288));
    let body = wire::request(&req, Protocol::Responses, false).unwrap();
    assert_eq!(
        body["reasoning"],
        json!({"effort":"xhigh","summary":"auto"})
    );
    assert!(body.get("thinking_budget").is_none());
}

#[test]
fn responses_terminal_preserves_phase_explicit_false_and_retry_advice() {
    let mut stream = StreamState::new(Protocol::Responses);
    stream.event(&json!({"type":"response.output_item.done","output_index":0,"item":{"type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"working"}]}}).to_string()).unwrap();
    let events = stream.event(&json!({"type":"response.completed","response":{"status":"completed","end_turn":false,"output":[]}}).to_string()).unwrap();
    let adk_core::ModelEvent::Complete { response } = events.last().unwrap() else {
        panic!()
    };
    assert_eq!(response.end_turn, Some(false));
    assert!(
        matches!(&response.items[0], adk_core::RunItem::PhasedMessage { phase, .. } if phase == "commentary")
    );
    assert!(stream.event("[DONE]").unwrap().is_empty());
    let mut failed = StreamState::new(Protocol::Responses);
    let error = failed.event(r#"{"type":"response.failed","response":{"status":"failed","error":{"code":"server_error","message":"secret-fixture"}}}"#).unwrap_err();
    assert!(retry_advice(&error).unwrap().should_retry);
    assert!(!format!("{error:?}").contains("secret-fixture"));
}

#[test]
fn responses_done_only_and_incomplete_output_are_usable() {
    let mut stream = StreamState::new(Protocol::Responses);
    let event = r#"{"type":"response.output_text.done","output_index":0,"text":"answer"}"#;
    assert!(
        matches!(&stream.event(event).unwrap()[0], adk_core::ModelEvent::TextDelta { delta } if delta == "answer")
    );
    assert!(stream.event(event).unwrap().is_empty());
    let body = json!({"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"end_turn":false,"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}]});
    assert_eq!(
        wire::response(&body, Protocol::Responses).unwrap().end_turn,
        Some(false)
    );
    let error = wire::response(
        &json!({"status":"completed","output":[]}),
        Protocol::Responses,
    )
    .unwrap_err();
    assert!(retry_advice(&error).unwrap().should_retry);
}

#[test]
fn anthropic_compaction_deltas_replace_ciphertext_and_reject_stopped_blocks() {
    let mut stream = StreamState::new(Protocol::Anthropic);
    for event in [
        json!({"type":"message_start","message":{"id":"m","usage":{}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"compaction","content":"a","encrypted_content":"old"}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"compaction_delta","content":"b","encrypted_content":"new"}}),
        json!({"type":"content_block_stop","index":0}),
    ] {
        stream.event(&event.to_string()).unwrap();
    }
    assert!(stream.event(r#"{"type":"content_block_delta","index":0,"delta":{"type":"compaction_delta","content":"late"}}"#).is_err());
    stream.event(r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#).unwrap();
    let events = stream.event(r#"{"type":"message_stop"}"#).unwrap();
    let adk_core::ModelEvent::Complete { response } = events.last().unwrap() else {
        panic!()
    };
    assert!(
        matches!(&response.items[0], adk_core::RunItem::Compaction { compaction } if compaction.content == "ab" && compaction.encrypted_content == "new" && compaction.created_by == "anthropic")
    );
}

#[test]
fn chat_reasoning_precedes_text_and_reused_tool_indices_keep_both_calls() {
    let mut stream = StreamState::new(Protocol::Chat);
    let events = stream.event(&json!({"choices":[{"index":0,"delta":{"content":"answer","reasoning_details":[{"summary":"why"}],"tool_calls":[{"index":0,"id":"first","function":{"name":"one","arguments":"{}"}}]}}]}).to_string()).unwrap();
    assert!(matches!(&events[0], adk_core::ModelEvent::ReasoningDelta { delta } if delta == "why"));
    assert!(matches!(&events[1], adk_core::ModelEvent::TextDelta { .. }));
    stream.event(&json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"second","function":{"name":"two","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}).to_string()).unwrap();
    let events = stream.event("[DONE]").unwrap();
    let adk_core::ModelEvent::Complete { response } = events.last().unwrap() else {
        panic!()
    };
    let ids: Vec<_> = response
        .items
        .iter()
        .filter_map(|item| match item {
            adk_core::RunItem::ToolCall { call } => Some(call.id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(ids, ["first", "second"]);
}

#[test]
fn executed_go_cost_catalog_and_retry_goldens_match() {
    let golden: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/providers/continuation.json"
    ))
    .unwrap();
    for vector in golden["openai_costs"].as_array().unwrap() {
        let usage = wire::usage(&vector["usage"], Protocol::Anthropic);
        let cost = adk_providers::cost::openai(vector["model"].as_str().unwrap(), &usage);
        assert_eq!(
            cost.is_some(),
            vector["known"].as_bool().unwrap(),
            "{}",
            vector["model"]
        );
        if let Some(cost) = cost {
            assert!(
                (cost - vector["cost"].as_f64().unwrap()).abs() < 1e-12,
                "{vector}: {cost}"
            );
        }
    }
    for vector in golden["anthropic_costs"].as_array().unwrap() {
        let usage = wire::usage(&vector["usage"], Protocol::Anthropic);
        let cost = adk_providers::cost::anthropic(vector["model"].as_str().unwrap(), &usage);
        assert!(
            (cost - vector["cost"].as_f64().unwrap()).abs() < 1e-12,
            "{vector}: {cost}"
        );
    }
    for vector in golden["retry_headers"].as_array().unwrap() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "retry-after-ms",
            vector["milliseconds"].as_str().unwrap().parse().unwrap(),
        );
        headers.insert(
            "retry-after",
            vector["seconds"].as_str().unwrap().parse().unwrap(),
        );
        assert_eq!(
            retry_after(&headers, SystemTime::UNIX_EPOCH).as_secs(),
            vector["capped_seconds"].as_u64().unwrap()
        );
    }
}

#[test]
fn fallback_models_verbosity_and_native_compaction_preserve_schema() {
    let mut input = request();
    input.settings.insert(
        "model_fallbacks".into(),
        json!(["fixture-model", " next ", "", "next", "vendor/last"]),
    );
    input
        .settings
        .insert("text_verbosity".into(), json!(" HIGH "));
    input
        .settings
        .insert("compaction_threshold".into(), json!(20000));
    input.output_schema = Some(json!({"type":"object","properties":{}}).try_into().unwrap());
    let chat = wire::request(&input, Protocol::Chat, false).unwrap();
    assert_eq!(
        chat["models"],
        json!(["fixture-model", "next", "vendor/last"])
    );
    assert!(chat.get("compaction_threshold").is_none());
    let responses = wire::request(&input, Protocol::Responses, false).unwrap();
    assert_eq!(responses["text"]["verbosity"], "high");
    assert_eq!(responses["text"]["format"]["type"], "json_schema");
    assert_eq!(
        responses["context_management"],
        json!([{"type":"compaction","compact_threshold":20000}])
    );
    assert!(responses.get("models").is_none());
}

#[test]
fn responses_done_only_tracking_is_per_content_part() {
    let mut stream = StreamState::new(Protocol::Responses);
    for part in 0..2 {
        let events = stream.event(&json!({"type":"response.output_text.done","output_index":0,"content_index":part,"text":"part"}).to_string()).unwrap();
        assert_eq!(events.len(), 1);
    }
}

#[test]
fn executed_go_chat_stream_matches_at_every_chunk_boundary() {
    let golden: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/providers/continuation.json"
    ))
    .unwrap();
    let source = golden["chat_sse"].as_str().unwrap().as_bytes();
    let mut expected = wire::response(&golden["chat_sse_response"], Protocol::Anthropic).unwrap();
    expected.usage.context_tokens = Some(expected.usage.input_tokens);
    // The Go client fixture is Anthropic-shaped; native raw data retains its protocol shape.
    expected.raw = Some(json!({
        "id": "chat-fixture",
        "choices": [{"finish_reason": "tool_calls", "message": {
            "content": "héllo", "reasoning_content": "why ", "reasoning_details": null,
            "reasoning_opaque": null, "tool_calls": [{"id": "call-fixture", "type": "function",
                "function": {"name": "lookup", "arguments": "{\"q\":1}"}}]
        }}],
        "usage": {"prompt_tokens": 100, "completion_tokens": 20,
            "prompt_tokens_details": {"cached_tokens": 70, "cache_write_tokens": 11}}
    }));
    for split in 0..=source.len() {
        let mut decoder = Decoder::default();
        let mut stream = StreamState::new(Protocol::Chat);
        let mut completed = Vec::new();
        for chunk in [&source[..split], &source[split..]] {
            for event in decoder.feed(chunk).unwrap() {
                for event in stream.event(&event.data).unwrap() {
                    if let adk_core::ModelEvent::Complete { response } = event {
                        completed.push(response);
                    }
                }
            }
        }
        decoder.finish().unwrap();
        assert_eq!(completed, vec![expected.clone()], "split {split}");
    }
}

#[test]
fn compacted_history_and_multimodal_tool_results_replay_without_reordering() {
    use adk_core::*;
    let mut input = request();
    input.input = wire::response(
        &json!({"output":[
            {"type":"message","role":"user","content":[{"type":"input_text","text":"keep"}]},
            {"type":"function_call","call_id":"c","name":"lookup","arguments":"{}"},
            {"type":"function_call_output","call_id":"c","output":"result"},
            {"type":"compaction","id":"compact","encrypted_content":"opaque"}
        ]}),
        Protocol::Responses,
    )
    .unwrap()
    .items;
    let body = wire::request(&input, Protocol::Responses, false).unwrap();
    assert_eq!(body["input"][0]["role"], "user");
    assert_eq!(body["input"][2]["type"], "function_call_output");
    assert_eq!(body["input"][3]["encrypted_content"], "opaque");
    input.input = vec![RunItem::ToolResult {
        call_id: "c".into(),
        output: ToolOutput {
            content: vec![
                Content::Text {
                    text: "result".into(),
                },
                Content::Attachment {
                    media_type: "image/png".into(),
                    data: "aW1hZ2U=".into(),
                    detail: "low".into(),
                },
            ],
            is_error: false,
            should_pause: false,
        },
    }];
    for protocol in [Protocol::Chat, Protocol::Responses, Protocol::Anthropic] {
        let body = wire::request(&input, protocol, false).unwrap();
        let entries = if protocol == Protocol::Responses {
            &body["input"]
        } else {
            &body["messages"]
        };
        if protocol == Protocol::Anthropic {
            assert_eq!(
                entries[0]["content"][0]["content"][1]["source"]["data"],
                "aW1hZ2U="
            );
        } else {
            let offset = usize::from(protocol == Protocol::Chat);
            assert_eq!(entries[offset + 1]["role"], "user");
            assert_eq!(
                entries[offset + 1]["content"][0]["type"],
                if protocol == Protocol::Chat {
                    "image_url"
                } else {
                    "input_image"
                }
            );
        }
    }
}

#[test]
fn empty_responses_message_is_retryable_and_chat_keeps_explicit_false() {
    let error = wire::response(&json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"  "}]}]}), Protocol::Responses).unwrap_err();
    assert!(retry_advice(&error).unwrap().should_retry);
    let mut stream = StreamState::new(Protocol::Chat);
    stream.event(r#"{"end_turn":false,"metadata":{"fixture":true},"choices":[{"delta":{"content":"working"},"finish_reason":"stop"}]}"#).unwrap();
    let events = stream.event("[DONE]").unwrap();
    let adk_core::ModelEvent::Complete { response } = events.last().unwrap() else {
        panic!()
    };
    assert_eq!(response.end_turn, Some(false));
    assert_eq!(response.metadata["fixture"], true);
}

#[test]
fn executed_go_sparse_responses_stream_retains_deltas_at_every_boundary() {
    let golden: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/providers/continuation.json"
    ))
    .unwrap();
    let source = golden["responses_sparse_sse"].as_str().unwrap().as_bytes();
    let reference = &golden["responses_sparse_sse_response"];
    let mut expected = wire::response(reference, Protocol::Anthropic).unwrap();
    expected.usage.context_tokens = Some(expected.usage.input_tokens);
    // The Go client fixture is Anthropic-shaped; native raw data retains its protocol shape.
    expected.raw = Some(json!({
        "id": "responses-fixture", "model": "gpt-5.6", "end_turn": false, "status": "completed",
        "output": [
            {"type": "reasoning", "id": "reasoning-fixture", "encrypted_content": "opaque",
                "summary": [{"type": "summary_text", "text": "why"}]},
            {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "héllo"}]},
            {"type": "function_call", "id": "item-fixture", "call_id": "call-fixture", "name": "lookup", "arguments": "{\"q\":1}"}
        ],
        "usage": {"input_tokens": 100, "output_tokens": 20,
            "input_tokens_details": {"cached_tokens": 70, "cache_write_tokens": 11}}
    }));
    let adk_core::RunItem::Reasoning { reasoning } = &mut expected.items[0] else {
        panic!()
    };
    reasoning.id = reference["content"][0]["id"].as_str().unwrap().into();
    reasoning.encrypted_content = reference["content"][0]["encrypted_content"]
        .as_str()
        .unwrap()
        .into();
    for split in 0..=source.len() {
        let mut decoder = Decoder::default();
        let mut stream = StreamState::new(Protocol::Responses);
        let mut completed = Vec::new();
        for chunk in [&source[..split], &source[split..]] {
            for event in decoder.feed(chunk).unwrap() {
                for event in stream.event(&event.data).unwrap() {
                    if let adk_core::ModelEvent::Complete { response } = event {
                        completed.push(response);
                    }
                }
            }
        }
        decoder.finish().unwrap();
        assert_eq!(completed, vec![expected.clone()], "split {split}");
    }
}

#[test]
fn explicit_compaction_origin_is_not_relabelled_as_the_current_protocol() {
    for protocol in [Protocol::Responses, Protocol::Anthropic] {
        let block = json!({"type":"compaction","id":"c","created_by":"foreign-gateway","encrypted_content":"opaque","content":"retained context"});
        let body = if protocol == Protocol::Responses {
            json!({"output":[block]})
        } else {
            json!({"content":[block]})
        };
        let response = wire::response(&body, protocol).unwrap();
        assert!(
            matches!(&response.items[0], adk_core::RunItem::Compaction { compaction } if compaction.created_by == "foreign-gateway")
        );
        let mut input = request();
        input.input = response.items;
        let replay = wire::request(&input, protocol, false).unwrap();
        assert!(!replay.to_string().contains("opaque"));
        assert!(replay.to_string().contains("retained context"));
    }
}

#[test]
fn sparse_refusal_stream_has_one_prefix_and_is_not_empty_output() {
    for done_only in [false, true] {
        let mut stream = StreamState::new(Protocol::Responses);
        let events = if done_only {
            vec![json!({"type":"response.refusal.done","output_index":0,"refusal":"cannot help"})]
        } else {
            vec![
                json!({"type":"response.refusal.delta","output_index":0,"delta":"cannot "}),
                json!({"type":"response.refusal.delta","output_index":0,"delta":"help"}),
                json!({"type":"response.refusal.done","output_index":0,"refusal":"cannot help"}),
            ]
        };
        let mut visible = String::new();
        for event in events {
            for event in stream.event(&event.to_string()).unwrap() {
                if let adk_core::ModelEvent::TextDelta { delta } = event {
                    visible.push_str(&delta);
                }
            }
        }
        assert_eq!(visible, "The model refused to respond: cannot help");
        let events = stream
            .event(r#"{"type":"response.completed","response":{"status":"completed","output":[]}}"#)
            .unwrap();
        let adk_core::ModelEvent::Complete { response } = events.last().unwrap() else {
            panic!()
        };
        assert!(
            matches!(&response.items[0], adk_core::RunItem::Message { message } if message.content == vec![adk_core::Content::Text { text: visible }])
        );
    }
}

#[test]
fn responses_in_band_transport_error_is_retryable_and_redacted() {
    let mut stream = StreamState::new(Protocol::Responses);
    let error = stream
        .event(r#"{"type":"error","message":"secret-fixture","code":"unexpected_backend_error"}"#)
        .unwrap_err();
    assert!(retry_advice(&error).unwrap().should_retry);
    assert!(!format!("{error:?}").contains("secret-fixture"));
}

#[test]
fn responses_native_tool_outputs_and_missing_call_ids_match_baseline() {
    for kind in [
        "function_call",
        "web_search_call",
        "file_search_call",
        "code_interpreter_call",
        "mcp_call",
        "computer_call",
        "image_generation_call",
        "tool_search_call",
        "local_shell_call",
        "shell_call",
        "apply_patch_call",
        "custom_tool_call",
    ] {
        let mut item = json!({"type":kind});
        if kind == "function_call" {
            item["name"] = "lookup".into();
        }
        let response = wire::response(&json!({"output":[item]}), Protocol::Responses).unwrap();
        let adk_core::RunItem::ToolCall { call } = &response.items[0] else {
            panic!()
        };
        assert_eq!(
            call.id,
            if kind == "function_call" {
                "call_0".to_owned()
            } else {
                format!("call_{kind}_0")
            }
        );
        assert_eq!(call.arguments, json!({}));
        assert_eq!(
            call.name,
            if kind == "function_call" {
                "lookup"
            } else {
                kind
            }
        );
    }
}

#[test]
fn compatible_chat_multipart_narration_and_truncated_tool_arguments_match_reference() {
    let body = json!({"choices":[{"message":{"content":[{"type":"text","text":"working"}]},"finish_reason":"tool_calls"}]});
    let response = wire::response(&body, Protocol::Chat).unwrap();
    assert_eq!(response.end_turn, Some(true));
    assert_eq!(response.items.len(), 1);
    for protocol in [Protocol::Chat, Protocol::Responses] {
        let body = if protocol == Protocol::Chat {
            json!({"choices":[{"message":{"tool_calls":[{"function":{"name":"lookup","arguments":"{\"incomplete\":"}}]},"finish_reason":"length"}]})
        } else {
            json!({"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[{"type":"function_call","name":"lookup","arguments":"{\"incomplete\":"}]})
        };
        let response = wire::response(&body, protocol).unwrap();
        let adk_core::RunItem::ToolCall { call } = &response.items[0] else {
            panic!()
        };
        assert_eq!(call.id, "call_0");
        assert_eq!(call.arguments, json!({}));
    }
}

#[test]
fn provider_response_retains_whole_body_not_just_metadata() {
    for (protocol, mut body) in [
        (
            Protocol::Chat,
            json!({"choices": [{"message": {"content": "answer"}, "finish_reason": "stop"}]}),
        ),
        (
            Protocol::Responses,
            json!({"status": "completed", "output": [{"type": "message", "content": [{"type": "output_text", "text": "answer"}]}]}),
        ),
        (
            Protocol::Anthropic,
            json!({"content": [{"type": "text", "text": "answer"}], "stop_reason": "end_turn"}),
        ),
    ] {
        body["metadata"] = json!({"key": "value"});
        body["provider_extension"] = json!({"nested": [null, false, "extra"]});
        let response = wire::response(&body, protocol).unwrap();
        assert_eq!(response.usage.requests, 1);
        assert_eq!(response.raw.as_ref(), Some(&body));
        assert_eq!(
            response.metadata,
            body["metadata"].as_object().unwrap().clone()
        );
        assert_eq!(
            serde_json::from_value::<adk_core::ModelResponse>(
                serde_json::to_value(&response).unwrap()
            )
            .unwrap(),
            response
        );
    }
}
