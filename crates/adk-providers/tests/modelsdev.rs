use adk_core::Context;
use adk_providers::modelsdev::{Options, Resolver};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn server(
    body: String,
    status: u16,
) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let mut buffer = [0; 4096];
                let n = stream.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
            }
            assert!(
                String::from_utf8_lossy(&request)
                    .to_lowercase()
                    .contains("accept: application/json")
            );
            count.fetch_add(1, Ordering::SeqCst);
            let response = format!(
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (url, hits, task)
}

fn options(url: String) -> Options {
    Options {
        url,
        cache_path: None,
        ..Options::default()
    }
}

#[tokio::test]
async fn original_lookup_thresholds_and_prefix_regressions_match_go() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/modelsdev/observations.json"
    ))
    .unwrap();
    let body: serde_json::Map<String, serde_json::Value> = fixture["catalog"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(provider, models)| {
            let models: serde_json::Map<String, serde_json::Value> = models
                .as_object()
                .unwrap()
                .iter()
                .map(|(id, limits)| (id.clone(), serde_json::json!({"limit":limits})))
                .collect();
            (provider.clone(), serde_json::json!({"models":models}))
        })
        .collect();
    let (url, hits, task) = server(serde_json::to_string(&body).unwrap(), 200).await;
    let resolver = Resolver::new(options(url)).unwrap();
    let context = context();
    for case in fixture["resolved"].as_array().unwrap() {
        let provider = case["provider"].as_str().unwrap();
        let model = case["model"].as_str().unwrap();
        let result = resolver.lookup(&context, provider, model).await;
        assert_eq!(result.is_some(), case["ok"].as_bool().unwrap(), "{case}");
        if let Some(limits) = result {
            assert_eq!(
                serde_json::to_value(limits).unwrap(),
                case["limits"],
                "{case}"
            );
        }
        for (actual, expected) in [
            (
                resolver
                    .compaction_thresholds(&context, provider, model)
                    .await,
                &case["thresholds"],
            ),
            (
                resolver.resolve_thresholds(&context, provider, model).await,
                &case["routed"],
            ),
        ] {
            let projected = actual.map_or(serde_json::json!([0, 0, false]), |(a, b)| {
                serde_json::json!([a, b, true])
            });
            assert_eq!(projected, *expected, "{case}");
        }
    }
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn memory_disk_stale_fallback_and_cancelled_cache_hits() {
    let body = r#"{"openai":{"models":{"m":{"limit":{"context":100,"output":-3}}}}}"#;
    let (url, hits, task) = server(body.into(), 200).await;
    let path = std::env::temp_dir()
        .join(format!(
            "adk-modelsdev-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
        .join("catalog.json");
    let mut opts = options(url);
    opts.cache_path = Some(path.clone());
    let resolver = Arc::new(Resolver::new(opts.clone()).unwrap());
    let mut joins = Vec::new();
    for _ in 0..8 {
        let resolver = resolver.clone();
        joins.push(tokio::spawn(async move {
            resolver.lookup(&context(), "openai", "m").await
        }));
    }
    for join in joins {
        assert_eq!(join.await.unwrap().unwrap().context_tokens, 100);
    }
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let token = Arc::new(adk_runtime::CancellationToken::new());
    token.cancel();
    let cancelled = Context {
        cancellation: token,
        ..context()
    };
    assert!(resolver.lookup(&cancelled, "openai", "m").await.is_some());
    assert!(path.is_file());
    task.abort();
    let (url, failed_hits, failed_task) = server("down".into(), 500).await;
    opts.url = url;
    opts.ttl = Duration::ZERO;
    let stale = Resolver::new(opts.clone()).unwrap();
    assert_eq!(
        stale
            .lookup(&context(), "openai", "m")
            .await
            .unwrap()
            .output_tokens,
        -3
    );
    assert_eq!(failed_hits.load(Ordering::SeqCst), 1);
    assert!(stale.lookup(&cancelled, "openai", "m").await.is_some());
    assert_eq!(failed_hits.load(Ordering::SeqCst), 1);
    opts.cache_path = None;
    assert!(
        Resolver::new(opts)
            .unwrap()
            .lookup(&cancelled, "openai", "m")
            .await
            .is_none()
    );
    failed_task.abort();
    tokio::fs::remove_dir_all(path.parent().unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn parse_failure_is_not_cached() {
    let (url, hits, task) = server("not json".into(), 200).await;
    let resolver = Resolver::new(options(url)).unwrap();
    assert!(resolver.lookup(&context(), "openai", "m").await.is_none());
    assert!(resolver.lookup(&context(), "openai", "m").await.is_none());
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    task.abort();
}

fn context() -> Context {
    Context {
        run_id: "modelsdev-test".into(),
        cancellation: Arc::new(adk_runtime::CancellationToken::new()),
        deadline: None,
    }
}

#[cfg(feature = "runtime")]
#[tokio::test]
async fn native_compaction_adapter_rejects_signed_overflow_and_honors_cancellation() {
    use adk_runtime::CompactionModelResolver;
    let (url, _, task) = server(r#"{"openai":{"models":{"m":{"limit":{"context":100}},"overflow":{"limit":{"context":1152921504606846976}}}}}"#.into(), 200).await;
    let resolver = adk_providers::modelsdev::CompactionResolver {
        resolver: Arc::new(Resolver::new(options(url)).unwrap()),
        default_provider: "openai".into(),
    };
    assert_eq!(
        resolver.thresholds(&context(), "m").await.unwrap(),
        Some((90, 50))
    );
    assert_eq!(
        resolver.thresholds(&context(), "overflow").await.unwrap(),
        None
    );
    let token = Arc::new(adk_runtime::CancellationToken::new());
    token.cancel();
    let cancelled = Context {
        cancellation: token,
        ..context()
    };
    assert_eq!(
        resolver
            .thresholds(&cancelled, "m")
            .await
            .unwrap_err()
            .info
            .category,
        adk_core::ErrorCategory::Cancelled
    );
    task.abort();
}

#[tokio::test]
async fn custom_transport_timeout_and_deadline_report_miss() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut opts = options(format!("http://{}", listener.local_addr().unwrap()));
    opts.client = Some(
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(20))
            .build()
            .unwrap(),
    );
    let resolver = Resolver::new(opts).unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_secs(2),
            resolver.lookup(&context(), "openai", "m")
        )
        .await
        .unwrap()
        .is_none()
    );
    let expired = Context {
        deadline: Some(std::time::Instant::now()),
        ..context()
    };
    assert!(resolver.lookup(&expired, "openai", "m").await.is_none());
}
