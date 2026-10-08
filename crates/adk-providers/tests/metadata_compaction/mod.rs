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
        let metadata = resolver.lookup(&context(), model).await.unwrap();
        assert_eq!(metadata.is_some(), case["found"].as_bool().unwrap());
        if let Some(metadata) = metadata {
            assert_eq!(metadata.id, case["id"].as_str().unwrap());
            assert_eq!(
                metadata.compaction_defaults().is_some(),
                case["valid"].as_bool().unwrap()
            );
        }
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
    assert!(
        resolver
            .lookup(&context(), "gpt-custom")
            .await
            .unwrap()
            .is_none()
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

#[test]
fn catalog_shapes_and_nullable_fields_match_pinned_sdk() {
    for case in fixture()["catalog_cases"].as_array().unwrap() {
        let result = parse_model_metadata(case["body"].as_str().unwrap().as_bytes());
        assert_eq!(
            result.is_err(),
            case["error"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        if case["error"] == true {
            continue;
        }
        let models = result.unwrap();
        let picker: Vec<_> = picker_model_metadata(&models)
            .into_iter()
            .map(|model| model.id)
            .collect();
        assert_eq!(json!(picker), case["picker"], "{}", case["name"]);
        let expected = case["models"].as_array().unwrap();
        assert_eq!(models.len(), expected.len(), "{}", case["name"]);
        let defaults = case["defaults"].as_array().unwrap();
        assert_eq!(expected.len(), defaults.len());
        for ((model, expected), defaults) in models.iter().zip(expected).zip(defaults) {
            let actual = json!({
                "ID": model.id,
                "ContextWindow": model.context_window.unwrap_or(0),
                "MaxContextWindow": model.max_context_window.unwrap_or(0),
                "MaxOutputTokens": model.max_output_tokens.unwrap_or(0),
                "AutoCompactTokenLimit": model.auto_compact_token_limit.unwrap_or(0),
                "EffectiveContextWindowPercent": model.effective_context_window_percent.unwrap_or(0),
                "DisplayName": model.display_name,
                "Description": model.description,
                "Visibility": model.visibility,
                "Priority": model.priority.unwrap_or(0),
                "DefaultReasoningLevel": model.default_reasoning_level,
                "SupportedReasoningLevels": (!model.supported_reasoning_levels.is_empty()).then_some(&model.supported_reasoning_levels),
                "UpgradeModel": model.upgrade_model.as_deref().unwrap_or(""),
            });
            assert_eq!(&actual, expected, "{}", case["name"]);
            let expected_defaults = defaults["valid"].as_bool().unwrap().then(|| {
                (
                    defaults["trigger"].as_u64().unwrap(),
                    defaults["target"].as_u64().unwrap(),
                )
            });
            assert_eq!(
                model.compaction_defaults(),
                expected_defaults,
                "{}",
                case["name"]
            );
        }
    }
}

#[test]
fn metadata_thresholds_match_pinned_sdk_signed_boundary_grid() {
    let fixture = fixture();
    let cases = fixture["threshold_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 219);
    for case in cases {
        let metadata = ModelMetadata {
            id: case["id"].as_str().unwrap().into(),
            context_window: Some(case["context"].as_i64().unwrap()),
            max_context_window: Some(case["max_context"].as_i64().unwrap()),
            auto_compact_token_limit: Some(case["auto_limit"].as_i64().unwrap()),
            effective_context_window_percent: Some(case["percent"].as_i64().unwrap()),
            ..Default::default()
        };
        let expected = case["valid"].as_bool().unwrap().then(|| {
            (
                case["trigger"].as_u64().unwrap(),
                case["target"].as_u64().unwrap(),
            )
        });
        assert_eq!(metadata.compaction_defaults(), expected, "{case}");
    }
}

#[tokio::test]
async fn public_lookup_inactive_context_lifecycle_matches_pinned_sdk() {
    let fixture = fixture();
    for kind in ["cancelled", "expired"] {
        let (session, calls, server) = server(vec![(
            200,
            r#"{"models":[{"slug":"cached","context_window":10000}]}"#.into(),
        )]);
        let resolver = MetadataCompactionResolver::new(session);
        let mut inactive = context();
        if kind == "cancelled" {
            let token = Arc::new(CancellationToken::new());
            token.cancel();
            inactive.cancellation = token;
        } else {
            inactive.deadline = Some(std::time::Instant::now() - Duration::from_secs(1));
        }
        for case in fixture["lifecycle_cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case["context"] == kind)
        {
            let stage = case["stage"].as_str().unwrap();
            if stage == "retry" {
                tokio::time::pause();
                tokio::time::advance(Duration::from_secs(31)).await;
                tokio::time::resume();
            }
            let active = context();
            let ctx = if matches!(stage, "cold" | "warm" | "warm-missing") {
                &inactive
            } else {
                &active
            };
            let result = resolver
                .lookup(ctx, case["model"].as_str().unwrap())
                .await
                .unwrap();
            assert_eq!(
                result.is_some(),
                case["found"].as_bool().unwrap(),
                "{kind}/{stage}"
            );
            assert_eq!(
                result.map(|m| m.id).unwrap_or_default(),
                case["id"].as_str().unwrap(),
                "{kind}/{stage}"
            );
            assert_eq!(
                calls.load(Ordering::SeqCst),
                case["requests"].as_u64().unwrap() as usize,
                "{kind}/{stage}"
            );
        }
        assert_eq!(
            resolver.warnings(),
            vec![
                MetadataCompactionWarning::FetchFailed,
                MetadataCompactionWarning::MissingModel("missing".into())
            ]
        );
        assert_eq!(
            resolver
                .thresholds(&inactive, "cached")
                .await
                .unwrap_err()
                .info
                .category,
            if kind == "cancelled" {
                ErrorCategory::Cancelled
            } else {
                ErrorCategory::DeadlineExceeded
            }
        );
        server.join().unwrap();
    }
}
