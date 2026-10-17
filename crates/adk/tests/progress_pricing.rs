#![cfg(feature = "observability")]

use adk::{
    core::{Context, RunResult, RunStatus, Usage},
    progress::{
        ProgressTracker,
        runtime::{ProgressOptions, RuntimeProgressAdapter, UsdPolicy},
    },
    runtime::{
        CancellationToken,
        progress::{ProgressFrame, ProgressUsage, RuntimeProgress, UsageOrigin},
    },
    tracewriter::Span,
    tracing::TraceProcessor,
};
use std::sync::{Arc, Mutex};

#[test]
fn uncertain_nonzero_host_usd_is_retained_and_reported_separately() {
    let tracker = ProgressTracker::new();
    let adapter = RuntimeProgressAdapter::new(
        tracker.clone(),
        ProgressOptions {
            usd: UsdPolicy::HostUnitsAreUsd,
            ..Default::default()
        },
    );
    let context = Context {
        run_id: "pricing".into(),
        deadline: None,
        cancellation: Arc::new(CancellationToken::new()),
    };
    let usage = Usage {
        input_tokens: 13,
        output_tokens: 7,
        cache_read_tokens: 5,
        cache_creation_tokens: 3,
        ..Default::default()
    };
    adapter.usage(
        &context,
        ProgressFrame {
            agent_name: "agent",
            agent_model: "configured-label",
            turn: 1,
            task_id: None,
        },
        ProgressUsage {
            origin: UsageOrigin::Accepted,
            request_model: "actual",
            provider: "provider",
            canonical_model: "canonical",
            usage: &usage,
            host_cost: Some(0.125),
            host_cost_known: false,
        },
    );
    let snapshot = tracker.snapshot();
    assert_eq!(snapshot.cost_usd, 0.125);
    assert_eq!(snapshot.input_tokens, 13);
    assert_eq!(
        snapshot.model_usage.unwrap()["configured-label"].cache_read_input_tokens,
        5
    );
    assert_eq!(
        adapter.drain_diagnostics(),
        ["USD price coverage is uncertain"]
    );
}

#[derive(Default)]
struct Recorder(Mutex<Vec<Span>>);
impl TraceProcessor for Recorder {
    fn span_end(&self, span: &Span) {
        self.0.lock().unwrap().push(span.clone());
    }
}

#[test]
fn external_span_ownership_keeps_session_completion_in_ring_and_pending_events() {
    let recorder = Arc::new(Recorder::default());
    let tracker = ProgressTracker::new().with_tracing_processor(recorder.clone());
    let adapter = Arc::new(RuntimeProgressAdapter::new(
        tracker.clone(),
        ProgressOptions::default(),
    ));
    let scope = adapter.session(4, "working");
    scope.finish(
        &RunResult {
            metrics: None,
            status: RunStatus::Completed,
            final_output: None,
            final_output_is_raw_json: false,
            new_items: vec![],
            new_items_provenance: vec![],
            history: vec![],
            history_provenance: vec![],
            responses: vec![],
            usage: Usage::default(),
            pending_approvals: vec![],
            last_agent: None,
            guardrails: vec![],
        },
        "completed",
    );
    assert!(
        recorder.0.lock().unwrap().is_empty(),
        "external owner emits the span"
    );
    let events = tracker.drain_pending_events();
    assert_eq!(
        events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        ["session_start", "session_complete"]
    );
    assert_eq!(events[1].summary, "Session 4 complete: $0.0000, 0 turns");
    assert_eq!(tracker.snapshot().events, events);
}
