use super::*;

struct ReasoningComplete {
    calls: AtomicUsize,
    chunks: Vec<&'static str>,
    fail_first: bool,
    wait: bool,
}
impl Model for ReasoningComplete {
    fn provider(&self) -> &str {
        "offline"
    }
    fn retry_advice(&self, _: &Error) -> Option<ModelRetryAdvice> {
        Some(ModelRetryAdvice {
            should_retry: true,
            retry_after: Duration::ZERO,
            reason: "test".into(),
        })
    }
    fn complete<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        self.complete_with_observers(context, request, None, None)
    }
    fn complete_with_observers<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
        _: Option<ModelActivitySink>,
        reasoning: Option<ModelReasoningSink<'a>>,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(sink) = reasoning {
                for chunk in &self.chunks {
                    sink(chunk);
                }
            }
            if self.wait {
                return std::future::pending().await;
            }
            if self.fail_first && call == 0 {
                return Err(Error::new(ErrorCategory::Provider, "offline failure"));
            }
            Ok(answer())
        })
    }
}

#[tokio::test]
async fn completion_only_reasoning_is_ordered_correlated_and_not_streamed() {
    let model = Arc::new(ReasoningComplete {
        calls: AtomicUsize::new(0),
        chunks: vec!["", "let me", " think"],
        fail_first: false,
        wait: false,
    });
    let seen = Arc::new(Mutex::new(vec![]));
    let sink = Arc::new(Sink::default());
    let runner = Runner::new(
        AgentConfig::new("worker", ModelBinding::complete("offline", model.clone())),
        config(&seen, &sink),
    )
    .unwrap();
    let result = runner
        .run(context(&CancellationToken::new()), request(), sink.clone())
        .await
        .unwrap();
    assert_eq!(result.result.responses, vec![answer()]);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    let seen = seen.lock().unwrap();
    check_order(
        &seen,
        &[
            "start",
            "delta:let me",
            "delta: think",
            "returned",
            "end",
            "accepted",
        ],
    );
    assert!(
        seen.iter()
            .all(|event| event.record.id == seen[0].record.id)
    );
    assert_eq!(
        seen.last().unwrap().record.status,
        GenerationStatus::Completed
    );
    assert_eq!(seen.last().unwrap().record.response, Some(answer()));
    assert!(!sink.events.lock().unwrap().iter().any(|event| matches!(
        event,
        RunEvent::Model {
            event: ModelEvent::ReasoningDelta { .. }
        }
    )));
}

#[tokio::test]
async fn streamed_completion_only_binding_does_not_install_reasoning_observer() {
    let model = Arc::new(ReasoningComplete {
        calls: AtomicUsize::new(0),
        chunks: vec!["hidden"],
        fail_first: false,
        wait: false,
    });
    let seen = Arc::new(Mutex::new(vec![]));
    let sink = Arc::new(Sink::default());
    let runner = Runner::new(
        AgentConfig::new("worker", ModelBinding::complete("offline", model.clone())),
        config(&seen, &sink),
    )
    .unwrap();
    runner
        .stream(context(&CancellationToken::new()), request(), sink.clone())
        .finish()
        .await
        .unwrap();
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    check_order(
        &seen.lock().unwrap(),
        &["start", "returned", "end", "accepted"],
    );
    assert!(!sink.events.lock().unwrap().iter().any(|event| matches!(
        event,
        RunEvent::Model {
            event: ModelEvent::ReasoningDelta { .. }
        }
    )));
}

#[tokio::test]
async fn completion_only_reasoning_retry_depends_on_actual_visibility() {
    for enabled in [false, true] {
        for observed in [false, true] {
            for chunk in ["", "visible"] {
                let model = Arc::new(ReasoningComplete {
                    calls: AtomicUsize::new(0),
                    chunks: vec![chunk],
                    fail_first: true,
                    wait: false,
                });
                let seen = Arc::new(Mutex::new(vec![]));
                let sink = Arc::new(Sink::default());
                let mut cfg = config(&seen, &sink);
                cfg.blocking_reasoning = enabled;
                if !observed {
                    cfg.generation_observer = None;
                }
                let runner = Runner::new(
                    AgentConfig::new("worker", ModelBinding::complete("offline", model.clone())),
                    cfg,
                )
                .unwrap();
                let result = runner
                    .run(context(&CancellationToken::new()), request(), sink.clone())
                    .await;
                let visible = enabled && observed && !chunk.is_empty();
                assert_eq!(result.is_err(), visible);
                assert_eq!(
                    model.calls.load(Ordering::SeqCst),
                    if visible { 1 } else { 2 }
                );
                assert_eq!(sink.accepted.load(Ordering::SeqCst), usize::from(!visible));
                if visible {
                    check_order(
                        &seen.lock().unwrap(),
                        &["start", "delta:visible", "returned", "end"],
                    );
                }
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn completion_only_reasoning_remains_idle_bounded() {
    let model = Arc::new(ReasoningComplete {
        calls: AtomicUsize::new(0),
        chunks: vec!["partial"],
        fail_first: false,
        wait: true,
    });
    let seen = Arc::new(Mutex::new(vec![]));
    let sink = Arc::new(Sink::default());
    let mut cfg = config(&seen, &sink);
    cfg.model_idle_timeout = Some(Duration::from_secs(1));
    let runner = Runner::new(
        AgentConfig::new("worker", ModelBinding::complete("offline", model.clone())),
        cfg,
    )
    .unwrap();
    let error = runner
        .run(context(&CancellationToken::new()), request(), sink.clone())
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::DeadlineExceeded);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert_eq!(sink.accepted.load(Ordering::SeqCst), 0);
    check_order(
        &seen.lock().unwrap(),
        &["start", "delta:partial", "returned", "end"],
    );
}
