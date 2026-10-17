#![cfg(feature = "observability")]

use adk::{
    codec::dto::Usage,
    progress::{
        AgentMeta, HookEvent, HookResult, MAX_EVENTS, ProgressEvent, ProgressSnapshot,
        ProgressTracker, RunProgress,
    },
    tracewriter::{Span, SpanData},
    tracing::TraceProcessor,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[test]
fn concurrent_record_returns_only_after_its_callbacks_before_host_flush() {
    use std::sync::{Barrier, mpsc};
    use std::time::{Duration, Instant};
    struct Blocking {
        entered: mpsc::Sender<()>,
        release: Barrier,
        calls: Mutex<Vec<String>>,
    }
    impl TraceProcessor for Blocking {
        fn span_start(&self, span: &Span) {
            self.calls
                .lock()
                .unwrap()
                .push(format!("start:{}", span.name));
            if span.name == "api.retry" {
                self.entered.send(()).unwrap();
                self.release.wait();
            }
        }
        fn span_end(&self, span: &Span) {
            self.calls
                .lock()
                .unwrap()
                .push(format!("end:{}", span.name));
        }
        fn flush(&self) -> Result<(), adk::core::Error> {
            self.calls.lock().unwrap().push("flush".into());
            Ok(())
        }
    }
    let (entered_tx, entered_rx) = mpsc::channel();
    let processor = Arc::new(Blocking {
        entered: entered_tx,
        release: Barrier::new(2),
        calls: Mutex::new(Vec::new()),
    });
    let tracker = ProgressTracker::new().with_tracing_processor(processor.clone());
    let first = tracker.clone();
    let a = std::thread::spawn(move || first.record_api_retry("busy", 1, 1, 2));
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let second = tracker.clone();
    let output = processor.clone();
    let (returned_tx, returned_rx) = mpsc::channel();
    let b = std::thread::spawn(move || {
        second.record_session_complete("m", 0.0, 1, &Usage::default(), 1, "done");
        output.flush().unwrap();
        returned_tx.send(()).unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !tracker
        .snapshot()
        .events
        .iter()
        .any(|event| event.event_type == "session_complete")
    {
        assert!(
            Instant::now() < deadline,
            "second caller did not record its event"
        );
        std::thread::yield_now();
    }
    let returned_early = returned_rx.recv_timeout(Duration::from_millis(100)).is_ok();
    // Always release and join before asserting, including against the regression.
    processor.release.wait();
    a.join().unwrap();
    b.join().unwrap();
    assert!(
        !returned_early,
        "record returned before its callbacks were delivered"
    );
    assert_eq!(
        *processor.calls.lock().unwrap(),
        [
            "start:api.retry",
            "end:api.retry",
            "start:session",
            "end:session",
            "flush",
        ]
    );
}

fn usage() -> Usage {
    Usage {
        requests: 99,
        input_tokens: 10,
        output_tokens: 5,
        cache_read_tokens: 20,
        cache_create_tokens: 30,
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<(bool, Span)>>);
impl TraceProcessor for Recorder {
    fn span_start(&self, span: &Span) {
        self.0.lock().unwrap().push((true, span.clone()));
    }
    fn span_end(&self, span: &Span) {
        self.0.lock().unwrap().push((false, span.clone()));
    }
    fn flush(&self) -> Result<(), adk::core::Error> {
        panic!("progress must not flush host resources")
    }
}

#[test]
fn sdk_wire_names_null_map_arrays_signed_fields_and_float_rejection() {
    let tracker: RunProgress = ProgressTracker::new();
    assert_eq!(
        serde_json::to_value(tracker.snapshot()).unwrap(),
        json!({
            "CurrentStep":"", "SessionNumber":0, "LastActivity":"", "AgentCount":0,
            "ToolCallCount":0, "CostUsd":0, "InputTokens":0, "OutputTokens":0,
            "CacheReadInputTokens":0, "CacheCreationInputTokens":0, "ModelUsage":null,
            "ApiRetries":0, "Events":[]
        })
    );
    assert_eq!(
        serde_json::to_value(tracker.drain_pending_events()).unwrap(),
        json!([])
    );
    tracker.record_llm_usage("", -0.25, &usage());
    let value = serde_json::to_value(tracker.snapshot()).unwrap();
    assert_eq!(
        value["ModelUsage"][""],
        json!({"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":20,"cache_creation_input_tokens":30})
    );
    assert_eq!(
        serde_json::from_value::<ProgressSnapshot>(value).unwrap(),
        tracker.snapshot()
    );
    tracker.record_llm_usage("nan", f64::NAN, &Usage::default());
    assert!(serde_json::to_string(&tracker.snapshot()).is_err());
}

#[test]
fn event_timestamps_trim_fractional_zeroes_like_go() {
    for (input, expected) in [
        ("123450000", "12345"),
        ("100000000", "1"),
        ("123456789", "123456789"),
    ] {
        let event: ProgressEvent = serde_json::from_value(json!({"timestamp":format!("2026-01-02T03:04:05.{input}Z"),"eventType":"test","summary":"ok"})).unwrap();
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({"timestamp":format!("2026-01-02T03:04:05.{expected}Z"),"eventType":"test","summary":"ok"})
        );
    }
}

#[test]
fn session_reset_preserves_aggregates_and_owned_snapshots() {
    let tracker = ProgressTracker::new();
    tracker.set_session(-1, "implementing");
    tracker.record_llm_usage("a", 0.25, &usage());
    tracker.record_tool_use("Read", "", "", 0, "", "", "");
    tracker.record_assistant_text("full text");
    tracker.record_assistant_thinking("full thinking");
    tracker.record_api_retry("rate", 1, 1, 2);
    tracker.record_subagent_started("child", "", "desc", "Explore", "a", "shared", "prompt");
    let mut snapshot = tracker.snapshot();
    snapshot.events.clear();
    snapshot
        .model_usage
        .as_mut()
        .unwrap()
        .get_mut("a")
        .unwrap()
        .input_tokens = 999;
    assert_eq!(tracker.session_cost_usd(), 0.25);
    assert_eq!(tracker.session_tool_call_count(), 1);
    tracker.set_session(2, "starting");
    assert_eq!(tracker.session_cost_usd(), 0.0);
    assert_eq!(tracker.session_tool_call_count(), 0);
    let snapshot = tracker.snapshot();
    assert_eq!(
        (
            snapshot.cost_usd,
            snapshot.tool_call_count,
            snapshot.api_retries,
            snapshot.agent_count
        ),
        (0.25, 1, 1, 1)
    );
    assert_eq!(snapshot.model_usage.unwrap()["a"].input_tokens, 10);
    assert_eq!(snapshot.last_activity, "full text");
    assert_eq!(tracker.last_assistant_text(), "full text");
    assert_eq!(tracker.last_assistant_thinking(), "full thinking");
    assert!(tracker.agent_meta("child").is_some());
    assert_eq!(tracker.drain_pending_events().len(), 2);
}

#[test]
fn descendant_usage_and_tools_only_forward_and_completion_never_double_counts() {
    let parent = ProgressTracker::new().with_max_tool_result_bytes(-1);
    parent.set_session(4, "reviewing");
    parent.record_assistant_text("parent");
    let child = parent.child("child");
    let grandchild = child.child("grandchild");
    assert_eq!(child.session_number(), 4);
    assert_eq!(child.max_tool_result_bytes(), -1);
    assert_eq!(child.task_id(), "child");
    assert_eq!(child.current_step(), "");
    assert_eq!(child.snapshot().last_activity, "");
    child.record_llm_usage("a", 0.25, &usage());
    grandchild.record_llm_usage("b", 0.5, &usage());
    grandchild.record_tool_use("Bash", "git commit", "", 0, "", "", "");
    grandchild.record_assistant_text("grandchild");
    grandchild.record_api_retry("rate", 0, 1, 2);
    child.record_subagent_started("nested", "", "nested", "Explore", "a", "", "");
    child.record_session_complete("a", 100.0, 99, &usage(), 500, "error");
    parent.record_subagent_completed(
        "child",
        "failed",
        "failed",
        100.0,
        99,
        &usage(),
        "error",
        &[],
        &[],
    );
    assert_eq!(parent.snapshot().cost_usd, 0.75);
    assert_eq!(parent.snapshot().input_tokens, 20);
    assert_eq!(parent.snapshot().output_tokens, 10);
    assert_eq!(parent.snapshot().cache_read_input_tokens, 40);
    assert_eq!(parent.snapshot().cache_creation_input_tokens, 60);
    assert_eq!(parent.snapshot().model_usage.unwrap().len(), 2);
    assert_eq!(child.snapshot().input_tokens, 20);
    assert_eq!(grandchild.snapshot().input_tokens, 10);
    assert_eq!(parent.snapshot().tool_call_count, 1);
    assert_eq!(child.snapshot().tool_call_count, 1);
    assert_eq!(parent.current_step(), "reviewing");
    assert_eq!(parent.snapshot().api_retries, 0);
    assert_eq!(parent.snapshot().agent_count, 0);
    assert_eq!(parent.snapshot().events.len(), 1);
    assert_eq!(parent.last_assistant_text(), "parent");
    assert_eq!(child.last_assistant_text(), "");
    parent.set_session(5, "starting");
    assert_eq!(child.session_number(), 4);
    assert_eq!(child.session_cost_usd(), 0.75);
}

#[test]
fn token_arithmetic_wraps_and_every_model_is_retained() {
    let tracker = ProgressTracker::new();
    let maximum = Usage {
        input_tokens: i64::MAX,
        output_tokens: i64::MIN,
        cache_read_tokens: i64::MAX,
        cache_create_tokens: i64::MIN,
        ..Default::default()
    };
    tracker.record_llm_usage("wrap", 0.0, &maximum);
    tracker.record_llm_usage(
        "wrap",
        0.0,
        &Usage {
            input_tokens: 1,
            output_tokens: -1,
            cache_read_tokens: 1,
            cache_create_tokens: -1,
            ..Default::default()
        },
    );
    let snapshot = tracker.snapshot();
    assert_eq!(
        (
            snapshot.input_tokens,
            snapshot.output_tokens,
            snapshot.cache_read_input_tokens,
            snapshot.cache_creation_input_tokens
        ),
        (i64::MIN, i64::MAX, i64::MIN, i64::MAX)
    );
    assert_eq!(snapshot.model_usage.unwrap()["wrap"].input_tokens, i64::MIN);
    for model in 0..100 {
        tracker.record_llm_usage(&model.to_string(), 0.0, &Usage::default());
    }
    assert_eq!(tracker.snapshot().model_usage.unwrap().len(), 101);
}

#[test]
fn exact_case_sensitive_step_inference_and_event_order() {
    for (initial, tool, command, expected) in [
        ("", "LSP", "", "exploring"),
        ("starting", "lsp", "", "starting"),
        ("reviewing", "LSP", "", "reviewing"),
        ("starting", "Read", "", "starting"),
        ("exploring", "Write", "", "implementing"),
        ("reviewing", "Edit", "", "reviewing"),
        ("committing", "Write", "", "committing"),
        ("starting", "write", "", "starting"),
        ("", "Bash", "git diff", "reviewing"),
        ("implementing", "Bash", "git diff", "implementing"),
        ("committing", "Bash", "git diff", "committing"),
        ("", "bash", "git commit", ""),
        ("", "Bash", "GIT COMMIT", ""),
        ("reviewing", "Bash", "echo git addendum", "committing"),
        ("starting", "Bash", "git diff; git commit", "committing"),
    ] {
        let tracker = ProgressTracker::new();
        tracker.set_session(1, initial);
        tracker.record_tool_use(tool, command, "", 0, "", "", "");
        assert_eq!(
            tracker.current_step(),
            expected,
            "{initial} {tool} {command}"
        );
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.last_activity, format!("Tool: {tool}"));
        assert_eq!(
            snapshot.events.len(),
            if initial == expected { 1 } else { 2 }
        );
        if initial != expected {
            assert_eq!(
                snapshot.events[1].summary,
                format!("Step: {expected} (via {tool})")
            );
        }
        tracker.set_step(expected);
        assert_eq!(tracker.snapshot().events.len(), snapshot.events.len());
    }
}

#[test]
fn event_ring_and_independent_unbounded_session_drain() {
    let tracker = ProgressTracker::new();
    for i in 0..30 {
        tracker.set_session(i, "starting");
        tracker.record_session_complete("", 0.25, 2, &Usage::default(), 1, "done");
    }
    let snapshot = tracker.snapshot();
    assert_eq!(snapshot.events.len(), MAX_EVENTS);
    assert_eq!(snapshot.events[0].summary, "Session 20 started: starting");
    let pending = tracker.drain_pending_events();
    assert_eq!(pending.len(), 60);
    assert_eq!(pending[59].summary, "Session 29 complete: $0.2500, 2 turns");
    assert_eq!(pending[59], snapshot.events[19]);
    assert!(tracker.drain_pending_events().is_empty());
    assert_eq!(tracker.snapshot(), snapshot);
}

#[test]
fn full_text_thinking_and_source_summary_normalization() {
    let tracker = ProgressTracker::new();
    let text = format!(" \n{}\nend \n", "界".repeat(121));
    tracker.record_assistant_text(&text);
    tracker.record_assistant_thinking("  thoughts\nkept  ");
    assert_eq!(tracker.last_assistant_text(), text);
    assert_eq!(tracker.last_assistant_thinking(), "  thoughts\nkept  ");
    assert_eq!(
        tracker.snapshot().last_activity,
        format!("{}...", "界".repeat(117))
    );
    assert!(tracker.snapshot().events.is_empty());
    tracker.record_lifecycle_event("", " \nhello\nworld \n");
    assert_eq!(tracker.snapshot().last_activity, "hello world");
    assert_eq!(tracker.snapshot().events[0].summary, " \nhello\nworld \n");
    assert_eq!(tracker.snapshot().events[0].event_type, "lifecycle");
    tracker.record_lifecycle_event("empty", "");
    assert_eq!(tracker.snapshot().last_activity, "hello world");
    tracker.record_tool_result("", "Bash", " hello\nworld ", true, 0, "", "");
    assert_eq!(
        tracker.snapshot().last_activity,
        "Tool Bash failed: hello world"
    );
    tracker.record_tool_result("", "Bash", "", false, 0, "", "");
    assert_eq!(tracker.snapshot().last_activity, "Tool Bash completed");
    tracker.write_result("failed", "https://pr", " bad\nnews ", "test");
    assert_eq!(
        tracker.snapshot().last_activity,
        "status=failed pr=https://pr failed_step=test error=bad news"
    );
    tracker.write_result("", "", "", "");
    assert_eq!(tracker.snapshot().last_activity, "run finished");
    let before = tracker.snapshot();
    tracker.record_hook_decision(&HookEvent::default(), &HookResult::default());
    assert_eq!(tracker.snapshot(), before);
    tracker.record_system_init("m", "cwd", &[], 1, &[], "mode");
    assert_eq!(tracker.current_step(), "starting");
    assert_eq!(
        tracker.snapshot().events.last().unwrap().summary,
        "Agent session initialized"
    );
    assert!(tracker.drain_pending_events().is_empty());
}

#[test]
fn metadata_is_owned_spawn_count_cumulative_and_cleanup_exact() {
    let tracker = ProgressTracker::new();
    for status in ["Completed", "cancelled", "completed", "failed", "stopped"] {
        tracker.record_subagent_started("a", "", "desc", "Explore", "model", "shared", "prompt");
        assert_eq!(
            tracker.agent_meta("a"),
            Some(AgentMeta {
                description: "desc".into(),
                subagent_type: "Explore".into(),
                model: "model".into(),
                isolation: "shared".into(),
                prompt: "prompt".into()
            })
        );
        tracker.agent_meta("a").unwrap().description.clear();
        tracker.record_subagent_progress("a", -2, -4, -1, "Read");
        assert_eq!(
            tracker.snapshot().last_activity,
            "Subagent a progress: -2 tools, -4 tokens, last tool Read"
        );
        tracker.record_subagent_completed(
            "a",
            status,
            " done\nnow ",
            99.0,
            8,
            &usage(),
            "stop",
            &[],
            &[],
        );
        assert_eq!(
            tracker.agent_meta("a").is_none(),
            matches!(status, "completed" | "failed" | "stopped")
        );
    }
    assert_eq!(tracker.snapshot().agent_count, 5);
    assert_eq!(tracker.snapshot().tool_call_count, 0);
    assert_eq!(tracker.snapshot().input_tokens, 0);
    assert_eq!(tracker.snapshot().last_activity, "done now");
}

#[test]
fn structural_spans_pair_fallback_and_copy_child_settings() {
    let processor = Arc::new(Recorder::default());
    let tracker = ProgressTracker::new().with_tracing_processor(processor.clone());
    tracker.set_root_span_id("phase-1");
    let child = tracker.child("child");
    tracker.set_root_span_id("phase-2");
    tracker.set_tracing_processor(None);
    child.record_subagent_started("a", "", "desc", "Explore", "m", "shared", "prompt");
    child.set_root_span_id("phase-3");
    std::thread::sleep(std::time::Duration::from_millis(2));
    child.record_subagent_completed(
        "a",
        "completed",
        " result ",
        0.5,
        3,
        &usage(),
        "stop",
        &["read".into()],
        &["write".into()],
    );
    child.record_subagent_completed(
        "missing",
        "failed",
        "oops",
        0.0,
        0,
        &Usage::default(),
        "error",
        &[],
        &[],
    );
    child.record_session_complete("m", 0.5, 3, &usage(), -20, "stop");
    child.record_api_retry("rate", -3, -1, -2);
    child.record_compact_boundary(100, 20, "ignored");
    let records = processor.0.lock().unwrap();
    assert_eq!(records.len(), 10);
    for pair in records.chunks_exact(2) {
        assert!(pair[0].0);
        assert!(!pair[1].0);
        assert_eq!(pair[0].1.id, pair[1].1.id);
        assert_eq!(pair[0].1.start_time, pair[1].1.start_time);
        assert!(pair[1].1.end_time >= pair[1].1.start_time);
    }
    assert_eq!(records[0].1.parent_id, "phase-1");
    assert_eq!(records[2].1.parent_id, "phase-3");
    let Some(SpanData::Subagent(data)) = &records[1].1.data else {
        panic!("subagent")
    };
    assert!(data.duration_ms >= 1);
    assert_eq!(
        (data.cost_usd, data.num_turns, data.total_tokens),
        (0.5, 3, 15)
    );
    assert_eq!(
        (data.cache_read_tokens, data.cache_creation_tokens),
        (20, 30)
    );
    assert_eq!(
        (&*data.prompt, &*data.result_text, &*data.isolation),
        ("prompt", "result", "shared")
    );
    assert_eq!(data.files_read, ["read"]);
    assert_eq!(data.files_written, ["write"]);
    assert_eq!(records[2].1.name, "subagent.");
    let Some(SpanData::Session(data)) = &records[5].1.data else {
        panic!("session")
    };
    assert_eq!(
        (
            data.duration_ms,
            data.input_tokens,
            data.cache_creation_input_tokens
        ),
        (-20, 10, 30)
    );
    assert!(matches!(
        &records[7].1.data,
        Some(SpanData::Retry {
            retry_after_ms: -3,
            attempt: -1,
            max_retries: -2,
            ..
        })
    ));
    assert!(matches!(
        &records[9].1.data,
        Some(SpanData::Compaction {
            tokens_before: 100,
            tokens_after: 20
        })
    ));
    assert_eq!(child.snapshot().cost_usd, 0.0);
}

#[test]
fn callbacks_can_reenter_and_remain_in_record_order() {
    struct Reentrant {
        tracker: Mutex<Option<ProgressTracker>>,
        records: Recorder,
    }
    impl TraceProcessor for Reentrant {
        fn span_start(&self, span: &Span) {
            self.records.span_start(span);
            let tracker = self.tracker.lock().unwrap().clone().unwrap();
            let _ = tracker.snapshot();
            if span.name == "session" {
                tracker.record_api_retry("nested", 0, 1, 2);
            }
        }
        fn span_end(&self, span: &Span) {
            self.records.span_end(span);
        }
    }
    let processor = Arc::new(Reentrant {
        tracker: Mutex::new(None),
        records: Recorder::default(),
    });
    let tracker = ProgressTracker::new().with_tracing_processor(processor.clone());
    *processor.tracker.lock().unwrap() = Some(tracker.clone());
    tracker.record_session_complete("", 0.0, 0, &Usage::default(), 0, "");
    let records = processor.records.0.lock().unwrap();
    assert_eq!(
        records
            .iter()
            .map(|(start, span)| (*start, span.name.as_str()))
            .collect::<Vec<_>>(),
        [
            (true, "session"),
            (false, "session"),
            (true, "api.retry"),
            (false, "api.retry")
        ]
    );
    processor.tracker.lock().unwrap().take();
    tracker.set_tracing_processor(None);
}

#[test]
fn clones_share_state_children_retain_parent_and_concurrent_counts_are_exact() {
    let parent = ProgressTracker::new();
    let child = parent.child("a");
    let clone = parent.clone();
    drop(parent);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..100 {
                    child.record_llm_usage("m", 0.25, &usage());
                    child.record_tool_use("Read", "", "", 0, "", "", "");
                }
            });
        }
    });
    assert_eq!(clone.snapshot().input_tokens, 8000);
    assert_eq!(clone.snapshot().cost_usd, 200.0);
    assert_eq!(clone.snapshot().tool_call_count, 800);
    assert_eq!(clone.current_step(), "");
    assert_eq!(clone.snapshot().last_activity, "");
}
