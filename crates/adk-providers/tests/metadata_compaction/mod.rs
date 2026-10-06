use super::*;
use adk_providers::runtime::{MetadataCompactionResolver, MetadataCompactionWarning};
use adk_runtime::{CompactionModelResolver, compaction::LocalCompactionPolicy};
use std::{
    io::{Read, Write},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../../fixtures/metadata-compaction/observations.json"
    ))
    .unwrap()
}
fn server(
    responses: Vec<(u16, String)>,
) -> (Arc<Session>, Arc<AtomicUsize>, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let task = std::thread::spawn(move || {
        for (status, body) in responses {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|v| v == b"\r\n\r\n") {
                let mut buffer = [0; 2048];
                let n = socket.read(&mut buffer).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
            }
            let request = String::from_utf8(request).unwrap().to_lowercase();
            assert!(request.starts_with("get /v1/models "));
            assert!(request.contains("authorization: bearer catalog-initial"));
            count.fetch_add(1, Ordering::SeqCst);
            write!(socket, "HTTP/1.1 {status} fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    (Arc::new(session(&endpoint, AuthMode::ApiKey)), calls, task)
}

#[tokio::test]
async fn lookup_matches_pinned_sdk_and_success_is_cached() {
    let fixture = fixture();
    let (session, calls, server) = server(vec![(200, fixture["catalog"].as_str().unwrap().into())]);
    let resolver = MetadataCompactionResolver::new(session);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(resolver.durable_key().is_none());
    for case in fixture["cases"].as_array().unwrap() {
        let model = case["model"].as_str().unwrap();
        let expected = if case["valid"] == true {
            (
                case["trigger"].as_u64().unwrap(),
                case["target"].as_u64().unwrap(),
            )
        } else {
            let fallback = LocalCompactionPolicy::for_model(model);
            (fallback.trigger_tokens, fallback.target_tokens)
        };
        assert_eq!(
            resolver.thresholds(&context(), model).await.unwrap(),
            Some(expected),
            "{model}"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            case["requests"].as_u64().unwrap() as usize
        );
    }
    assert_eq!(resolver.warnings().len(), 2);
    server.join().unwrap();
}

#[tokio::test]
async fn failed_fetch_cools_down_then_retries_without_leaking_error_body() {
    let fixture = fixture();
    let (session, calls, server) = server(vec![
        (500, "private-secret-body".into()),
        (200, fixture["catalog"].as_str().unwrap().into()),
    ]);
    let resolver = MetadataCompactionResolver::new(session);
    for _ in 0..2 {
        assert_eq!(
            resolver.thresholds(&context(), "gpt-custom").await.unwrap(),
            Some((180_000, 100_000))
        );
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        fixture["retry"]["requests_before_cooldown"]
            .as_u64()
            .unwrap() as usize
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::time::resume();
    assert_eq!(
        resolver.thresholds(&context(), "gpt-custom").await.unwrap(),
        Some((9000, 5000))
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        fixture["retry"]["requests_after_cooldown"]
            .as_u64()
            .unwrap() as usize
    );
    assert_eq!(
        resolver.warnings(),
        vec![MetadataCompactionWarning::FetchFailed]
    );
    server.join().unwrap();
}

#[tokio::test]
async fn concurrent_calls_share_fetch_and_cancelled_call_does_not_poison_cache() {
    let fixture = fixture();
    let (session, calls, server) = server(vec![(200, fixture["catalog"].as_str().unwrap().into())]);
    let resolver = MetadataCompactionResolver::new(session);
    let token = Arc::new(CancellationToken::new());
    token.cancel();
    let cancelled = Context {
        cancellation: token,
        ..context()
    };
    assert!(resolver.thresholds(&cancelled, "gpt-custom").await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let ctx = context();
    let (a, b) = tokio::join!(
        resolver.thresholds(&ctx, "gpt-custom"),
        resolver.thresholds(&ctx, "other/gpt-custom")
    );
    assert_eq!(a.unwrap(), Some((9000, 5000)));
    assert_eq!(b.unwrap(), Some((9000, 5000)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    server.join().unwrap();
}

#[tokio::test]
async fn hung_endpoint_is_bounded_and_caller_cancellation_remains_an_error() {
    for cancel in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let resolver = Arc::new(MetadataCompactionResolver::new(Arc::new(session(
            &endpoint,
            AuthMode::ApiKey,
        ))));
        let token = Arc::new(CancellationToken::new());
        let ctx = Context {
            cancellation: token.clone(),
            ..context()
        };
        let pending = resolver.clone();
        let task = tokio::spawn(async move { pending.thresholds(&ctx, "gpt-custom").await });
        let (_socket, _) = listener.accept().await.unwrap();
        if cancel {
            token.cancel();
            assert_eq!(
                task.await.unwrap().unwrap_err().info.category,
                ErrorCategory::Cancelled
            );
        } else {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(16)).await;
            assert_eq!(task.await.unwrap().unwrap(), Some((180_000, 100_000)));
            tokio::time::resume();
            assert_eq!(
                resolver.warnings(),
                vec![MetadataCompactionWarning::FetchFailed]
            );
        }
    }
}

#[tokio::test]
async fn independent_session_scopes_never_share_catalogs() {
    let first = fixture()["catalog"].as_str().unwrap().to_owned();
    let second = first.replace("10000", "30000");
    let (a, _, server_a) = server(vec![(200, first)]);
    let (b, _, server_b) = server(vec![(200, second)]);
    let a = MetadataCompactionResolver::new(a);
    let b = MetadataCompactionResolver::new(b);
    let ctx = context();
    let (a, b) = tokio::join!(
        a.thresholds(&ctx, "gpt-custom"),
        b.thresholds(&ctx, "gpt-custom")
    );
    assert_eq!(a.unwrap(), Some((9000, 5000)));
    assert_eq!(b.unwrap(), Some((27000, 15000)));
    server_a.join().unwrap();
    server_b.join().unwrap();
}
