use adk_core::{BoxFuture, Context, Error, ErrorCategory};
use adk_providers::{auth::*, error::RequestFailure, material, oauth::decode_response};
use adk_runtime::CancellationToken;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};
use tokio::sync::Notify;

const NOW: u64 = 1_800_000_000;
fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}
fn context() -> Context {
    Context {
        run_id: "auth-parity".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn scope(mode: AuthMode) -> Scope {
    Scope::new(
        "fixture-route",
        "https://provider.example.test",
        Some("account".into()),
        mode,
    )
    .unwrap()
}
fn seed() -> Material {
    Material {
        access_token: Secret::new("fixture-access"),
        refresh_token: Some(Secret::new("fixture-refresh")),
        id_token: None,
        account: Some("account".into()),
        email: Some("fixture@example.test".into()),
        expires_at: Some(at(NOW + 60)),
        last_refresh: Some(at(NOW - 8 * 86400)),
        revision: 1,
    }
}
fn jwt(claims: Value) -> String {
    format!("e30.{}.fixture", URL_SAFE_NO_PAD.encode(claims.to_string()))
}
fn mode(name: &str) -> AuthMode {
    match name {
        "openai" => AuthMode::OpenAiOAuth,
        "anthropic" => AuthMode::AnthropicOAuth,
        "copilot" => AuthMode::CopilotOAuth,
        _ => panic!("unknown fixture mode"),
    }
}
#[derive(Default)]
struct Store {
    value: Mutex<Option<Material>>,
    scopes: Mutex<Vec<Scope>>,
    writes: AtomicUsize,
}
impl Store {
    fn new(value: Material) -> Arc<Self> {
        Arc::new(Self {
            value: Mutex::new(Some(value)),
            ..Self::default()
        })
    }
    fn rotate(&self, edit: impl FnOnce(&mut Material)) {
        let mut guard = self.value.lock().unwrap();
        let value = guard.as_mut().unwrap();
        edit(value);
        value.revision += 1;
    }
}
impl CredentialStore for Store {
    fn load<'a>(
        &'a self,
        _: &'a Context,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async move {
            self.scopes.lock().unwrap().push(scope.clone());
            Ok(self.value.lock().unwrap().as_ref().unwrap().clone())
        })
    }
    fn replace<'a>(
        &'a self,
        _: &'a Context,
        scope: &'a Scope,
        revision: u64,
        mut material: Material,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async move {
            self.scopes.lock().unwrap().push(scope.clone());
            let mut guard = self.value.lock().unwrap();
            if guard.as_ref().unwrap().revision != revision {
                return Ok(false);
            }
            material.revision = revision + 1;
            *guard = Some(material);
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        })
    }
}
type Action = dyn Fn(usize, Material) -> Result<Material, Error> + Send + Sync;
struct Exchange {
    calls: AtomicUsize,
    action: Box<Action>,
    pause: Option<(Arc<Notify>, Arc<Notify>)>,
}
impl Exchange {
    fn new(
        action: impl Fn(usize, Material) -> Result<Material, Error> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            action: Box::new(action),
            pause: None,
        })
    }
    fn paused(
        action: impl Fn(usize, Material) -> Result<Material, Error> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            action: Box::new(action),
            pause: Some((Arc::new(Notify::new()), Arc::new(Notify::new()))),
        })
    }
    fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}
impl Refresh for Exchange {
    fn refresh<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        material: Material,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some((entered, release)) = &self.pause {
                entered.notify_one();
                release.notified().await;
            }
            (self.action)(call, material)
        })
    }
}
fn success(_: usize, mut value: Material) -> Result<Material, Error> {
    value.access_token = Secret::new("refreshed-access");
    value.last_refresh = Some(at(NOW));
    value.expires_at = None;
    Ok(value)
}
fn failure(_: usize, _: Material) -> Result<Material, Error> {
    Err(RequestFailure::http(401, &reqwest::header::HeaderMap::new(), at(NOW)).into_error())
}
fn session(
    mode: AuthMode,
    store: Arc<Store>,
    exchange: Arc<Exchange>,
    clock: Arc<AtomicU64>,
) -> Session {
    Session::new(scope(mode), store, exchange)
        .unwrap()
        .with_clock(Arc::new(move || at(clock.load(Ordering::SeqCst))))
}
fn clock() -> Arc<AtomicU64> {
    Arc::new(AtomicU64::new(NOW))
}

#[test]
fn response_fixtures_match_baseline_material_rules() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/providers/auth-responses.json"
    ))
    .unwrap();
    assert_eq!(
        fixture["baseline"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    for case in fixture["cases"].as_array().unwrap() {
        let result = decode_response(
            &scope(mode(case["mode"].as_str().unwrap())),
            seed(),
            case["body"].to_string().as_bytes(),
            at(NOW),
        );
        if case["error"].as_bool() == Some(true) {
            let error = result.unwrap_err();
            assert!(!format!("{error:?} {error}").contains("sensitive-fixture"));
            continue;
        }
        let value = result.unwrap();
        assert_eq!(
            value.access_token.expose(),
            case["access"].as_str().unwrap(),
            "{}",
            case["name"]
        );
        assert_eq!(
            value.refresh_token.unwrap().expose(),
            case["refresh"].as_str().unwrap()
        );
        assert_eq!(value.account.as_deref(), case["account"].as_str());
        assert_eq!(value.email.as_deref(), case["email"].as_str());
        assert_eq!(value.expires_at, case["expiry"].as_u64().map(at));
        assert_eq!(value.last_refresh, Some(at(NOW)));
        assert_eq!(value.revision, 1);
    }
}

#[test]
fn openai_refresh_preserves_established_account_and_derives_only_missing_identity() {
    let id = jwt(json!({"https://api.openai.com/auth":{"chatgpt_account_id":"new-account"}}));
    let access = jwt(json!({"exp":NOW + 3600}));
    let body = json!({"access_token":access,"id_token":id});
    let mut unscoped = scope(AuthMode::OpenAiOAuth);
    unscoped.account = None;
    let retained =
        decode_response(&unscoped, seed(), body.to_string().as_bytes(), at(NOW)).unwrap();
    assert_eq!(retained.account.as_deref(), Some("account"));
    assert_eq!(retained.id_token.unwrap().expose(), id);
    assert_eq!(retained.expires_at, Some(at(NOW + 3600)));
    let mut missing = seed();
    missing.account = None;
    let derived = decode_response(
        &unscoped,
        missing.clone(),
        body.to_string().as_bytes(),
        at(NOW),
    )
    .unwrap();
    assert_eq!(derived.account.as_deref(), Some("new-account"));
    assert!(
        decode_response(
            &scope(AuthMode::OpenAiOAuth),
            missing.clone(),
            body.to_string().as_bytes(),
            at(NOW)
        )
        .is_err()
    );
    assert!(
        decode_response(
            &unscoped,
            missing,
            br#"{"access_token":"fixture-access"}"#,
            at(NOW)
        )
        .is_err()
    );
}

#[test]
fn response_diagnostics_are_bounded_secret_safe_and_modes_do_not_cross() {
    for raw in [
        b"sensitive-fixture".as_slice(),
        b"[]",
        b"null",
        br#"{"refresh_token":"sensitive-fixture"}"#,
    ] {
        let err = decode_response(&scope(AuthMode::OpenAiOAuth), seed(), raw, at(NOW)).unwrap_err();
        assert!(!format!("{err:?} {err}").contains("sensitive-fixture"));
        assert!(err.source.is_none());
    }
    assert!(
        decode_response(
            &scope(AuthMode::CopilotOAuth),
            seed(),
            &vec![b'a'; 1024 * 1024 + 1],
            at(NOW)
        )
        .is_err()
    );
    assert!(
        decode_response(
            &scope(AuthMode::ApiKey),
            seed(),
            br#"{"access_token":"fixture"}"#,
            at(NOW)
        )
        .is_err()
    );
}

#[tokio::test]
async fn openai_retrying_transport_does_not_burn_opaque_refresh_chain() {
    let store = Store::new(seed());
    let exchange = Exchange::new(success);
    let session = session(AuthMode::OpenAiOAuth, store, exchange.clone(), clock());
    let context = context();
    let original = session.material_for_request(&context).await.unwrap();
    assert_eq!(exchange.count(), 0);
    session
        .reject(&context, &original.access_token)
        .await
        .unwrap();
    assert_eq!(
        session
            .material_for_request(&context)
            .await
            .unwrap()
            .access_token
            .expose(),
        "refreshed-access"
    );
    assert_eq!(exchange.count(), 1);
}

#[tokio::test]
async fn openai_direct_and_retrying_jwt_refresh_boundaries_are_distinct() {
    for (proactive, until_expiry, expected) in
        [(true, 301, 0), (true, 300, 1), (false, 1, 0), (false, 0, 1)]
    {
        let mut value = seed();
        value.access_token = Secret::new(jwt(json!({"exp":NOW + until_expiry})));
        let exchange = Exchange::new(success);
        let session = session(
            AuthMode::OpenAiOAuth,
            Store::new(value),
            exchange.clone(),
            clock(),
        );
        if proactive {
            session.material(&context()).await.unwrap();
        } else {
            session.material_for_request(&context()).await.unwrap();
        }
        assert_eq!(exchange.count(), expected);
    }
    let exchange = Exchange::new(success);
    session(
        AuthMode::OpenAiOAuth,
        Store::new(seed()),
        exchange.clone(),
        clock(),
    )
    .material(&context())
    .await
    .unwrap();
    assert_eq!(exchange.count(), 1);
}

#[tokio::test]
async fn copilot_grace_cooldown_expiry_and_rotation_are_revision_scoped() {
    let store = Store::new(seed());
    let exchange = Exchange::new(failure);
    let clock = clock();
    let session = session(
        AuthMode::CopilotOAuth,
        store.clone(),
        exchange.clone(),
        clock.clone(),
    );
    let context = context();
    for _ in 0..3 {
        assert_eq!(
            session
                .material(&context)
                .await
                .unwrap()
                .access_token
                .expose(),
            "fixture-access"
        );
    }
    assert_eq!(exchange.count(), 1);
    clock.store(NOW + 14, Ordering::SeqCst);
    session.material(&context).await.unwrap();
    assert_eq!(exchange.count(), 1);
    clock.store(NOW + 15, Ordering::SeqCst);
    session.material(&context).await.unwrap();
    assert_eq!(exchange.count(), 2);
    store.rotate(|value| value.refresh_token = Some(Secret::new("host-rotated-refresh")));
    session.material(&context).await.unwrap();
    assert_eq!(exchange.count(), 3);
    clock.store(NOW + 60, Ordering::SeqCst);
    assert!(session.material(&context).await.is_err());
    assert_eq!(exchange.count(), 4);
    assert!(session.material(&context).await.is_err());
    assert_eq!(exchange.count(), 4);
}

#[tokio::test]
async fn fallback_never_serves_rejected_expired_or_missing_credentials() {
    for mode in [AuthMode::CopilotOAuth, AuthMode::AnthropicOAuth] {
        for (token, expiry, rejected) in [
            ("fixture-access", NOW + 60, true),
            ("fixture-access", NOW, false),
            ("", NOW + 60, false),
        ] {
            let mut value = seed();
            value.access_token = Secret::new(token);
            value.expires_at = Some(at(expiry));
            let session = session(mode, Store::new(value), Exchange::new(failure), clock());
            if rejected {
                session
                    .reject(&context(), &Secret::new(token))
                    .await
                    .unwrap();
            }
            assert!(session.material(&context()).await.is_err());
        }
    }
}

#[tokio::test]
async fn static_tokens_and_anonymous_never_exchange_or_fall_back_to_other_modes() {
    for mode in [
        AuthMode::ApiKey,
        AuthMode::Anonymous,
        AuthMode::CopilotOAuth,
        AuthMode::AnthropicOAuth,
        AuthMode::OpenAiOAuth,
    ] {
        let mut value = seed();
        value.refresh_token = None;
        value.expires_at = Some(at(NOW - 1));
        let exchange = Exchange::new(failure);
        let session = session(mode, Store::new(value), exchange.clone(), clock());
        session.material(&context()).await.unwrap();
        assert_eq!(exchange.count(), 0);
    }
}

#[tokio::test]
async fn concurrent_refreshes_use_one_exchange_and_scope_qualified_cas() {
    let store = Store::new(seed());
    let exchange = Exchange::paused(success);
    let session = session(
        AuthMode::AnthropicOAuth,
        store.clone(),
        exchange.clone(),
        clock(),
    );
    let ctx = context();
    let (entered, release) = exchange.pause.as_ref().unwrap();
    let control = async {
        entered.notified().await;
        release.notify_one();
    };
    let (a, b, c, ()) = tokio::join!(
        session.material(&ctx),
        session.material(&ctx),
        session.material(&ctx),
        control
    );
    for value in [a, b, c] {
        assert_eq!(value.unwrap().access_token.expose(), "refreshed-access");
    }
    assert_eq!(exchange.count(), 1);
    assert_eq!(store.writes.load(Ordering::SeqCst), 1);
    assert!(
        store
            .scopes
            .lock()
            .unwrap()
            .iter()
            .all(|s| s == session.scope())
    );
}

#[tokio::test]
async fn external_rotation_wins_both_successful_cas_and_failed_refresh_races() {
    for fails in [false, true] {
        let store = Store::new(seed());
        let exchange = Exchange::paused(move |n, value| {
            if fails {
                failure(n, value)
            } else {
                success(n, value)
            }
        });
        let session = session(
            AuthMode::AnthropicOAuth,
            store.clone(),
            exchange.clone(),
            clock(),
        );
        let (entered, release) = exchange.pause.as_ref().unwrap();
        let rotate = async {
            entered.notified().await;
            store.rotate(|value| {
                value.access_token = Secret::new("host-access");
                value.expires_at = None;
            });
            release.notify_one();
        };
        let ctx = context();
        let (result, ()) = tokio::join!(session.material(&ctx), rotate);
        assert_eq!(result.unwrap().access_token.expose(), "host-access");
        assert_eq!(store.writes.load(Ordering::SeqCst), 0);
        assert_eq!(exchange.count(), 1);
    }
}

#[tokio::test]
async fn openai_retries_new_host_refresh_token_once_only_after_http_rejection() {
    for status_error in [true, false] {
        let store = Store::new(seed());
        let rotated = store.clone();
        let exchange = Exchange::new(move |call, value| {
            if call == 0 {
                rotated.rotate(|v| v.refresh_token = Some(Secret::new("host-refresh")));
                if status_error {
                    failure(call, value)
                } else {
                    Err(Error::new(ErrorCategory::Provider, "transport failure"))
                }
            } else {
                assert_eq!(
                    value.refresh_token.as_ref().unwrap().expose(),
                    "host-refresh"
                );
                success(call, value)
            }
        });
        let session = session(AuthMode::OpenAiOAuth, store, exchange.clone(), clock());
        let result = session.material(&context()).await;
        assert_eq!(result.is_ok(), status_error);
        assert_eq!(exchange.count(), if status_error { 2 } else { 1 });
    }
}

#[tokio::test]
async fn repeated_host_rotation_does_not_create_an_unbounded_refresh_loop() {
    let store = Store::new(seed());
    let rotated = store.clone();
    let exchange = Exchange::new(move |call, value| {
        rotated.rotate(|v| v.refresh_token = Some(Secret::new(format!("host-refresh-{call}"))));
        failure(call, value)
    });
    let session = session(AuthMode::OpenAiOAuth, store, exchange.clone(), clock());
    assert!(session.material(&context()).await.is_err());
    assert_eq!(exchange.count(), 2);
}

#[tokio::test]
async fn stale_401_cannot_clear_rejection_and_unchanged_refresh_is_not_reused() {
    let mut value = seed();
    value.expires_at = None;
    let exchange = Exchange::new(|_, v| Ok(v));
    let session = session(
        AuthMode::CopilotOAuth,
        Store::new(value),
        exchange.clone(),
        clock(),
    );
    session
        .reject(&context(), &Secret::new(" fixture-access "))
        .await
        .unwrap();
    session
        .reject(&context(), &Secret::new("older-access"))
        .await
        .unwrap();
    assert_eq!(
        session
            .material(&context())
            .await
            .unwrap_err()
            .info
            .category,
        ErrorCategory::PermissionDenied
    );
    assert_eq!(exchange.count(), 1);
}

#[tokio::test]
async fn cancellation_of_waiter_does_not_cancel_owner_or_trigger_cooldown() {
    let store = Store::new(seed());
    let exchange = Exchange::paused(success);
    let session = session(
        AuthMode::CopilotOAuth,
        store.clone(),
        exchange.clone(),
        clock(),
    );
    let cancel = Arc::new(CancellationToken::new());
    let waiter = Context {
        cancellation: cancel.clone(),
        ..context()
    };
    let ctx = context();
    let (entered, release) = exchange.pause.as_ref().unwrap();
    let wait_and_cancel = async {
        entered.notified().await;
        let waiting = session.material(&waiter);
        let cancellation = async {
            tokio::task::yield_now().await;
            cancel.cancel();
        };
        let (result, ()) = tokio::join!(waiting, cancellation);
        assert_eq!(result.unwrap_err().info.category, ErrorCategory::Cancelled);
        release.notify_one();
    };
    let (result, ()) = tokio::join!(session.material(&ctx), wait_and_cancel);
    assert_eq!(result.unwrap().access_token.expose(), "refreshed-access");
    assert_eq!(exchange.count(), 1);
    assert_eq!(store.writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancelled_exchange_releases_gate_without_recording_copilot_failure() {
    let store = Store::new(seed());
    let exchange = Exchange::paused(success);
    let session = session(
        AuthMode::CopilotOAuth,
        store.clone(),
        exchange.clone(),
        clock(),
    );
    let cancel = Arc::new(CancellationToken::new());
    let ctx = Context {
        cancellation: cancel.clone(),
        ..context()
    };
    let (entered, release) = exchange.pause.as_ref().unwrap();
    let cancellation = async {
        entered.notified().await;
        cancel.cancel();
    };
    let (result, ()) = tokio::join!(session.material(&ctx), cancellation);
    assert_eq!(result.unwrap_err().info.category, ErrorCategory::Cancelled);
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
    release.notify_one();
    session.material(&context()).await.unwrap();
    assert_eq!(exchange.count(), 2);
    assert_eq!(store.writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn account_mismatch_is_rejected_before_persist_and_never_echoes_tokens() {
    let store = Store::new(seed());
    let exchange = Exchange::new(|_, mut value| {
        value.access_token = Secret::new("sensitive-fixture");
        value.account = Some("other-account".into());
        Ok(value)
    });
    let session = session(AuthMode::AnthropicOAuth, store.clone(), exchange, clock());
    let err = session.material(&context()).await.unwrap_err();
    assert_eq!(err.info.category, ErrorCategory::PermissionDenied);
    assert!(!format!("{err:?}").contains("sensitive-fixture"));
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
}

#[test]
fn headers_and_cache_isolate_route_endpoint_mode_account_and_secret() {
    let original = seed();
    let original_scope = scope(AuthMode::OpenAiOAuth);
    let first = cache_scope(&original_scope, &original, "prompt");
    for (route, endpoint, mode) in [
        (
            "other",
            "https://provider.example.test",
            AuthMode::OpenAiOAuth,
        ),
        (
            "fixture-route",
            "https://other.example.test",
            AuthMode::OpenAiOAuth,
        ),
        (
            "fixture-route",
            "https://provider.example.test",
            AuthMode::ApiKey,
        ),
    ] {
        let other = Scope::new(route, endpoint, Some("account".into()), mode).unwrap();
        assert_ne!(cache_scope(&other, &original, "prompt"), first);
    }
    let mut mismatched = original.clone();
    mismatched.account = Some("other-account".into());
    assert!(headers(&original_scope, &mismatched, false).is_err());
    assert!(cache_scope(&original_scope, &mismatched, "prompt").is_empty());
    let mut empty = original.clone();
    empty.access_token = Secret::new(" ");
    assert!(headers(&original_scope, &empty, false).is_err());
    let headers = headers(&original_scope, &original, false).unwrap();
    assert!(headers["authorization"].is_sensitive());
    assert!(headers["chatgpt-account-id"].is_sensitive());
    let debug = format!(
        "{headers:?} {:?}",
        material::serialize(AuthMode::OpenAiOAuth, &original, at(NOW)).unwrap()
    );
    assert!(!debug.contains("fixture-access"));
    assert!(!debug.contains("fixture-refresh"));
}

#[tokio::test]
async fn failed_exchange_grace_rechecks_expiry_at_completion() {
    for mode in [AuthMode::AnthropicOAuth, AuthMode::CopilotOAuth] {
        let clock = clock();
        let during_exchange = clock.clone();
        let exchange = Exchange::new(move |n, value| {
            during_exchange.store(NOW + 60, Ordering::SeqCst);
            failure(n, value)
        });
        assert!(
            session(mode, Store::new(seed()), exchange, clock)
                .material(&context())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn deadline_before_auth_does_not_load_or_exchange_credentials() {
    let store = Store::new(seed());
    let exchange = Exchange::new(success);
    let session = session(
        AuthMode::CopilotOAuth,
        store.clone(),
        exchange.clone(),
        clock(),
    );
    let ctx = Context {
        deadline: Some(std::time::Instant::now()),
        ..context()
    };
    assert_eq!(
        session.material(&ctx).await.unwrap_err().info.category,
        ErrorCategory::DeadlineExceeded
    );
    assert_eq!(exchange.count(), 0);
    assert!(store.scopes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn isolated_route_sessions_do_not_share_locks_rejections_or_cooldowns() {
    let blocked = Exchange::paused(success);
    let first = session(
        AuthMode::AnthropicOAuth,
        Store::new(seed()),
        blocked.clone(),
        clock(),
    );
    let second_exchange = Exchange::new(success);
    let second_store = Store::new(seed());
    let second_scope = Scope::new(
        "other-route",
        "https://other.example.test",
        Some("account".into()),
        AuthMode::CopilotOAuth,
    )
    .unwrap();
    let second = Session::new(second_scope, second_store.clone(), second_exchange.clone())
        .unwrap()
        .with_clock(Arc::new(|| at(NOW)));
    let (entered, release) = blocked.pause.as_ref().unwrap();
    let independent = async {
        entered.notified().await;
        second.material(&context()).await.unwrap();
        assert_eq!(second_exchange.count(), 1);
        release.notify_one();
    };
    let ctx = context();
    let (result, ()) = tokio::join!(first.material(&ctx), independent);
    result.unwrap();
    assert!(
        second_store
            .scopes
            .lock()
            .unwrap()
            .iter()
            .all(|scope| scope == second.scope())
    );
}

#[tokio::test]
async fn openai_material_requires_account_even_when_refresh_is_unavailable() {
    let mut value = seed();
    value.account = None;
    value.refresh_token = None;
    let mut unscoped = scope(AuthMode::OpenAiOAuth);
    unscoped.account = None;
    let session = Session::new(unscoped, Store::new(value), Exchange::new(failure)).unwrap();
    assert_eq!(
        session
            .material_for_request(&context())
            .await
            .unwrap_err()
            .info
            .category,
        ErrorCategory::PermissionDenied
    );
}

#[tokio::test]
async fn host_can_disable_local_refresh_without_disabling_external_rotation() {
    let store = Store::new(seed());
    let exchange = Exchange::new(success);
    let session = session(
        AuthMode::OpenAiOAuth,
        store.clone(),
        exchange.clone(),
        clock(),
    );
    session.disable_refresh();
    let ctx = context();
    let old = session.material(&ctx).await.unwrap();
    assert_eq!(exchange.count(), 0);
    session.reject(&ctx, &old.access_token).await.unwrap();
    assert!(session.material(&ctx).await.is_err());
    store.rotate(|v| v.access_token = Secret::new("external-access"));
    assert_eq!(
        session.material(&ctx).await.unwrap().access_token.expose(),
        "external-access"
    );
    assert_eq!(exchange.count(), 0);
}

#[tokio::test]
async fn refresh_failure_cannot_resurrect_credentials_revoked_by_the_host() {
    for mode in [AuthMode::AnthropicOAuth, AuthMode::CopilotOAuth] {
        let store = Store::new(seed());
        let rotated = store.clone();
        let exchange = Exchange::new(move |call, value| {
            rotated.rotate(|current| {
                current.access_token = Secret::new("");
                current.refresh_token = None;
            });
            failure(call, value)
        });
        let session = session(mode, store, exchange, clock());
        assert!(session.material(&context()).await.is_err());
    }
}
