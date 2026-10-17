use super::*;

#[derive(Default)]
struct HeartbeatModel {
    calls: AtomicUsize,
    stall: bool,
    fail_first: bool,
}
impl Model for HeartbeatModel {
    fn provider(&self) -> &str {
        "heartbeat"
    }
    fn complete<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        self.complete_with_activity(context, request, None)
    }
    fn complete_with_activity<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
        activity: Option<ModelActivitySink>,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            let mut stream = self
                .stream_with_activity(context, request, activity)
                .await?;
            let Some(ModelEvent::Complete { response }) = stream.next().await? else {
                panic!("missing completion")
            };
            Ok(response)
        })
    }
}
impl StreamingModel for HeartbeatModel {
    fn stream<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        self.stream_with_activity(context, request, None)
    }
    fn stream_with_activity<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
        activity: Option<ModelActivitySink>,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            let attempt = self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(HeartbeatStream {
                activity,
                finished: false,
                stall: self.stall,
                fail: self.fail_first && attempt == 0,
            }) as Box<dyn ModelStream>)
        })
    }
}
struct HeartbeatStream {
    activity: Option<ModelActivitySink>,
    finished: bool,
    stall: bool,
    fail: bool,
}
impl ModelStream for HeartbeatStream {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async move {
            if self.finished {
                return Ok(None);
            }
            self.finished = true;
            for _ in 0..3 {
                tokio::time::sleep(Duration::from_secs(5)).await;
                if let Some(activity) = &self.activity {
                    activity();
                }
            }
            if self.stall {
                std::future::pending::<()>().await;
            }
            if self.fail {
                return Err(provider_error());
            }
            Ok(Some(ModelEvent::Complete {
                response: answer("ok"),
            }))
        })
    }
}

#[tokio::test(start_paused = true)]
async fn transport_activity_resets_idle_in_all_model_call_modes() {
    for streaming_binding in [false, true] {
        for streaming_run in [false, true] {
            for blocking_reasoning in [false, true] {
                let model = Arc::new(HeartbeatModel::default());
                let binding = if streaming_binding {
                    ModelBinding::streaming("model", model.clone())
                } else {
                    ModelBinding::complete("model", model.clone())
                };
                let config = RunnerConfig {
                    model_idle_timeout: Some(Duration::from_secs(6)),
                    blocking_reasoning,
                    ..Default::default()
                };
                let runner = Runner::new(AgentConfig::new("agent", binding), config).unwrap();
                let start = tokio::time::Instant::now();
                let result = if streaming_run {
                    runner
                        .stream(context(), request(1), Arc::new(TestHost::default()))
                        .finish()
                        .await
                } else {
                    runner
                        .run(context(), request(1), Arc::new(TestHost::default()))
                        .await
                }
                .unwrap();
                assert_eq!(result.result.final_output, Some(json!("ok")));
                assert_eq!(model.calls.load(Ordering::SeqCst), 1);
                assert_eq!(start.elapsed(), Duration::from_secs(15));
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn stopped_activity_still_times_out_from_the_last_progress() {
    let model = Arc::new(HeartbeatModel {
        stall: true,
        ..Default::default()
    });
    let config = RunnerConfig {
        model_idle_timeout: Some(Duration::from_secs(6)),
        ..Default::default()
    };
    let runner = Runner::new(
        AgentConfig::new("agent", ModelBinding::streaming("model", model)),
        config,
    )
    .unwrap();
    let start = tokio::time::Instant::now();
    let error = runner
        .stream(context(), request(1), Arc::new(TestHost::default()))
        .finish()
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::DeadlineExceeded);
    assert_eq!(start.elapsed(), Duration::from_secs(21));
}

#[tokio::test(start_paused = true)]
async fn activity_never_extends_absolute_deadlines_or_prevents_cancellation() {
    for streaming in [false, true] {
        for deadline in [false, true] {
            let model = Arc::new(HeartbeatModel::default());
            let binding = if streaming {
                ModelBinding::streaming("model", model)
            } else {
                ModelBinding::complete("model", model)
            };
            let runner = Runner::new(
                AgentConfig::new("agent", binding),
                RunnerConfig {
                    model_idle_timeout: Some(Duration::from_secs(6)),
                    ..Default::default()
                },
            )
            .unwrap();
            let signal = Arc::new(CancellationToken::new());
            let mut ctx = context();
            ctx.cancellation = signal.clone();
            if deadline {
                ctx.deadline =
                    Some((tokio::time::Instant::now() + Duration::from_secs(12)).into_std());
            }
            let start = tokio::time::Instant::now();
            let (result, ()) = tokio::join!(
                async {
                    if streaming {
                        runner
                            .stream(ctx, request(1), Arc::new(TestHost::default()))
                            .finish()
                            .await
                    } else {
                        runner
                            .run(ctx, request(1), Arc::new(TestHost::default()))
                            .await
                    }
                },
                async {
                    tokio::time::sleep(Duration::from_secs(12)).await;
                    if !deadline {
                        signal.cancel();
                    }
                }
            );
            assert_eq!(
                result.err().unwrap().error.info.category,
                if deadline {
                    ErrorCategory::DeadlineExceeded
                } else {
                    ErrorCategory::Cancelled
                }
            );
            assert!(start.elapsed() >= Duration::from_secs(11));
            assert!(start.elapsed() <= Duration::from_secs(13));
        }
    }
}

#[tokio::test(start_paused = true)]
async fn previous_attempt_activity_cannot_keep_a_new_attempt_alive() {
    #[derive(Default)]
    struct StaleModel {
        calls: AtomicUsize,
        previous: Mutex<Option<ModelActivitySink>>,
    }
    impl Model for StaleModel {
        fn provider(&self) -> &str {
            "stale"
        }
        fn complete<'a>(
            &'a self,
            ctx: &'a Context,
            request: ModelRequest,
        ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
            self.complete_with_activity(ctx, request, None)
        }
        fn complete_with_activity<'a>(
            &'a self,
            _: &'a Context,
            _: ModelRequest,
            activity: Option<ModelActivitySink>,
        ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
            Box::pin(async move {
                if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    *self.previous.lock().unwrap() = activity;
                    return Err(provider_error());
                }
                let stale = self.previous.lock().unwrap().clone().unwrap();
                for _ in 0..3 {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    stale();
                }
                Ok(answer("must time out"))
            })
        }
    }
    let model = Arc::new(StaleModel::default());
    let config = RunnerConfig {
        model_idle_timeout: Some(Duration::from_secs(6)),
        retry: RetryPolicy {
            max_retries: 1,
            initial_delay: Duration::ZERO,
            ..Default::default()
        },
        ..Default::default()
    };
    let runner = Runner::new(
        AgentConfig::new("agent", ModelBinding::complete("model", model.clone())),
        config,
    )
    .unwrap();
    let start = tokio::time::Instant::now();
    let error = runner
        .run(context(), request(2), Arc::new(TestHost::default()))
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::DeadlineExceeded);
    assert_eq!(model.calls.load(Ordering::SeqCst), 2);
    assert_eq!(start.elapsed(), Duration::from_secs(6));
}

#[tokio::test(start_paused = true)]
async fn activity_is_not_visible_output_and_does_not_disable_retry() {
    let model = Arc::new(HeartbeatModel {
        fail_first: true,
        ..Default::default()
    });
    let config = RunnerConfig {
        model_idle_timeout: Some(Duration::from_secs(6)),
        retry: RetryPolicy {
            max_retries: 1,
            initial_delay: Duration::ZERO,
            ..Default::default()
        },
        ..Default::default()
    };
    let runner = Runner::new(
        AgentConfig::new("agent", ModelBinding::streaming("model", model.clone())),
        config,
    )
    .unwrap();
    let host = Arc::new(TestHost::default());
    let result = runner
        .stream(context(), request(2), host.clone())
        .finish()
        .await
        .unwrap();
    assert_eq!(result.result.final_output, Some(json!("ok")));
    assert_eq!(model.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        host.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, RunEvent::Model { .. }))
            .count(),
        1
    );
}
