#![cfg(feature = "observability")]
use adk::{
    tracewriter::{Span, SpanData, Trace},
    tracing::*,
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, String, String)>>, Mutex<Vec<Trace>>);
impl TraceProcessor for Recorder {
    fn trace_start(&self, t: &Trace) {
        self.0
            .lock()
            .unwrap()
            .push(("trace_start".into(), t.id.clone(), String::new()));
    }
    fn trace_end(&self, t: &Trace) {
        self.0
            .lock()
            .unwrap()
            .push(("trace_end".into(), t.id.clone(), String::new()));
        self.1.lock().unwrap().push(t.clone());
    }
    fn span_start(&self, s: &Span) {
        self.0
            .lock()
            .unwrap()
            .push(("span_start".into(), s.id.clone(), s.parent_id.clone()));
    }
    fn span_end(&self, s: &Span) {
        self.0
            .lock()
            .unwrap()
            .push(("span_end".into(), s.id.clone(), s.parent_id.clone()));
    }
}
#[test]
fn shared_root_closes_once_after_late_child_and_updates_final_data() {
    let sink = Arc::new(Recorder::default());
    let scope = TraceSession::new("root", sink.clone());
    let shared = scope.clone();
    let root_id = scope.id();
    let parent = scope.span("parent", None);
    let parent_id = parent.id().to_owned();
    let mut child = parent.child("child", None);
    *child.data_mut() = Some(SpanData::Guardrail {
        guardrail_name: "policy".into(),
        triggered: true,
    });
    scope.finish();
    shared.finish();
    parent.finish();
    assert!(sink.1.lock().unwrap().is_empty());
    drop(child);
    let traces = sink.1.lock().unwrap();
    assert_eq!(traces.len(), 1);
    let trace = &traces[0];
    assert_eq!(trace.spans.len(), 2);
    assert_eq!(trace.spans[0].parent_id, root_id);
    assert_eq!(trace.spans[1].parent_id, parent_id);
    assert!(matches!(
        trace.spans[1].data,
        Some(SpanData::Guardrail {
            triggered: true,
            ..
        })
    ));
    assert!(trace.end_time >= trace.spans[1].end_time);
    let events = sink.0.lock().unwrap();
    assert_eq!(
        events.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(),
        [
            "trace_start",
            "span_start",
            "span_start",
            "span_end",
            "span_end",
            "trace_end"
        ]
    );
}
#[test]
fn fanout_preserves_registration_order_and_empty_composition_works() {
    struct Sink(usize, Arc<Mutex<Vec<(usize, &'static str)>>>);
    impl TraceProcessor for Sink {
        fn trace_start(&self, _: &Trace) {
            self.1.lock().unwrap().push((self.0, "start"));
        }
        fn trace_end(&self, _: &Trace) {
            self.1.lock().unwrap().push((self.0, "end"));
        }
    }
    let events = Arc::new(Mutex::new(vec![]));
    let processor = CompositeTraceProcessor(vec![
        Arc::new(Sink(1, events.clone())),
        Arc::new(Sink(2, events.clone())),
    ]);
    TraceSession::new("root", Arc::new(processor)).finish();
    assert_eq!(
        *events.lock().unwrap(),
        [(1, "start"), (2, "start"), (1, "end"), (2, "end")]
    );
    TraceSession::new("noop", Arc::new(CompositeTraceProcessor::default())).finish();
}
#[test]
fn concurrent_spans_keep_root_alive_without_processor_lock_reentrancy() {
    let sink = Arc::new(Recorder::default());
    let scope = TraceSession::new("root", sink.clone());
    let mut tasks = vec![];
    for _ in 0..8 {
        let shared = scope.clone();
        tasks.push(std::thread::spawn(move || {
            let span = shared.span("worker", None);
            shared.finish();
            span.finish();
        }));
    }
    scope.finish();
    for task in tasks {
        task.join().unwrap();
    }
    let traces = sink.1.lock().unwrap();
    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0].spans.len(), 8);
    let events = sink.0.lock().unwrap();
    for span in &traces[0].spans {
        let start = events
            .iter()
            .position(|e| e.0 == "span_start" && e.1 == span.id)
            .unwrap();
        let end = events
            .iter()
            .position(|e| e.0 == "span_end" && e.1 == span.id)
            .unwrap();
        assert!(start < end);
    }
    assert_eq!(events.last().unwrap().0, "trace_end");
}

#[tokio::test]
async fn runner_observer_owns_root_and_preserves_generation_parent() {
    runner_case(false).await;
}

#[tokio::test]
async fn dropping_run_future_closes_generation_before_root() {
    runner_case(true).await;
}

async fn runner_case(pending: bool) {
    use adk::core::*;
    use adk::runtime::{AgentConfig, CancellationToken, ModelBinding, Runner, RunnerConfig};
    struct NoopHost;
    impl Host for NoopHost {
        fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
            Box::pin(async { Ok(()) })
        }
        fn approve<'a>(
            &'a self,
            _: &'a Context,
            _: ApprovalRequest,
        ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
            Box::pin(async { panic!("unexpected approval") })
        }
    }
    struct Provider(bool);
    impl Model for Provider {
        fn provider(&self) -> &str {
            "fixture"
        }
        fn complete<'a>(
            &'a self,
            _: &'a Context,
            _: ModelRequest,
        ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
            Box::pin(async move {
                if self.0 {
                    std::future::pending::<()>().await;
                }
                Ok(ModelResponse {
                    response_id: None,
                    items: vec![],
                    usage: Usage::default(),
                    end_turn: Some(true),
                    metadata: Default::default(),
                    raw: None,
                })
            })
        }
    }
    let sink = Arc::new(Recorder::default());
    let scope = TraceSession::new("root", sink.clone());
    let agent = scope.span("agent", None);
    let parent = agent.id().to_owned();
    let runner = Runner::new(
        AgentConfig::new(
            "agent",
            ModelBinding::complete("fixture", Arc::new(Provider(pending))),
        ),
        RunnerConfig {
            generation_observer: Some(agent.generation_observer()),
            ..Default::default()
        },
    )
    .unwrap();
    scope.finish();
    agent.finish();
    assert!(sink.1.lock().unwrap().is_empty());
    let context = Context {
        run_id: "run".into(),
        cancellation: Arc::new(CancellationToken::default()),
        deadline: None,
    };
    let mut run = Box::pin(runner.run(
        context,
        RunRequest {
            input: vec![],
            policy: RunPolicy::default(),
        },
        Arc::new(NoopHost),
    ));
    if pending {
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(run.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
    } else {
        run.as_mut().await.unwrap();
    }
    drop(run);
    assert!(sink.1.lock().unwrap().is_empty());
    drop(runner);
    let traces = sink.1.lock().unwrap();
    assert_eq!(traces.len(), 1);
    let generation = traces[0]
        .spans
        .iter()
        .find(|s| matches!(s.data, Some(SpanData::Generation(_))))
        .unwrap();
    assert_eq!(generation.parent_id, parent);
    if let Some(SpanData::Generation(data)) = &generation.data {
        assert_eq!(data.success, !pending);
        assert_eq!(data.response.is_some(), !pending);
        assert_eq!(
            data.status,
            if pending { "interrupted" } else { "completed" }
        );
    }
}

#[test]
fn flush_visits_every_processor_and_retains_all_failures_in_order() {
    use adk::core::{Error, ErrorCategory};
    use std::error::Error as _;
    struct Sink(usize, bool, Arc<Mutex<Vec<usize>>>);
    impl TraceProcessor for Sink {
        fn flush(&self) -> Result<(), Error> {
            self.2.lock().unwrap().push(self.0);
            if self.1 {
                Err(Error::new(ErrorCategory::Host, self.0.to_string()))
            } else {
                Ok(())
            }
        }
    }
    let calls = Arc::new(Mutex::new(vec![]));
    let composite = CompositeTraceProcessor(vec![
        Arc::new(Sink(1, true, calls.clone())),
        Arc::new(Sink(2, false, calls.clone())),
        Arc::new(Sink(3, true, calls.clone())),
    ]);
    let error = composite.flush().unwrap_err();
    assert_eq!(*calls.lock().unwrap(), [1, 2, 3]);
    let failures = error
        .source()
        .unwrap()
        .downcast_ref::<FlushErrors>()
        .unwrap();
    assert_eq!(
        failures
            .errors
            .iter()
            .map(|e| e.info.message.as_str())
            .collect::<Vec<_>>(),
        ["1", "3"]
    );
    CompositeTraceProcessor::default().flush().unwrap();
}
