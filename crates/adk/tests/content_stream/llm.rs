use adk::{
    codec::{dto::ContentEvent, snapshots::to_go_json},
    content_stream::{ContentStream, LlmAttempt},
};
use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn llm_six_field_input_duplicates_aliases_without_normalizing() {
    for attempt in [0, 2, -1, i32::MIN, i32::MAX] {
        let stream = ContentStream::new(io::sink());
        let sub = stream.subscribe(1);
        stream.emit_llm_attempt(LlmAttempt {
            attempt,
            scope: " subagent\n".into(),
            agent_name: "helper😀".into(),
            model: "model\n".into(),
            task_id: "task-123".into(),
            status: "unrecognized status".into(),
        });
        let event = sub.receiver.try_recv().unwrap();
        let expected = ContentEvent {
            timestamp: event.timestamp.clone(),
            kind: "llm_attempt".into(),
            llm_attempt: attempt,
            attempt_number: attempt,
            llm_scope: " subagent\n".into(),
            scope: " subagent\n".into(),
            agent_name: "helper😀".into(),
            model: "model\n".into(),
            canonical_model: "model\n".into(),
            task_id: "task-123".into(),
            status: "unrecognized status".into(),
            attempt_status: "unrecognized status".into(),
            ..Default::default()
        };
        assert_eq!(to_go_json(&event).unwrap(), to_go_json(&expected).unwrap());
        assert!(sub.receiver.try_recv().is_err());
    }
}

#[test]
fn llm_alias_defaults_are_independent_and_one_way() {
    for mask in 0..16 {
        let stream = ContentStream::new(io::sink());
        let sub = stream.subscribe(1);
        stream.emit_llm_attempt(ContentEvent {
            kind: "wrong".into(),
            llm_attempt: if mask & 1 == 0 { 0 } else { -7 },
            attempt_number: i32::MIN,
            llm_scope: if mask & 2 == 0 { "" } else { " " }.into(),
            scope: "canonical scope".into(),
            status: if mask & 4 == 0 { "" } else { "legacy status" }.into(),
            attempt_status: "canonical status".into(),
            model: if mask & 8 == 0 { "" } else { "legacy model" }.into(),
            canonical_model: "canonical model".into(),
            ..Default::default()
        });
        let event = sub.receiver.try_recv().unwrap();
        assert_eq!(event.kind, "llm_attempt");
        assert_eq!(event.llm_attempt, if mask & 1 == 0 { i32::MIN } else { -7 });
        assert_eq!(event.attempt_number, i32::MIN);
        assert_eq!(
            event.llm_scope,
            if mask & 2 == 0 {
                "canonical scope"
            } else {
                " "
            }
        );
        assert_eq!(event.scope, "canonical scope");
        assert_eq!(
            event.status,
            if mask & 4 == 0 {
                "canonical status"
            } else {
                "legacy status"
            }
        );
        assert_eq!(event.attempt_status, "canonical status");
        assert_eq!(
            event.model,
            if mask & 8 == 0 {
                "canonical model"
            } else {
                "legacy model"
            }
        );
        assert_eq!(event.canonical_model, "canonical model");
    }

    let stream = ContentStream::new(io::sink());
    let sub = stream.subscribe(1);
    stream.emit_llm_attempt(ContentEvent {
        llm_attempt: 3,
        llm_scope: "legacy scope".into(),
        status: "legacy status".into(),
        model: "legacy model".into(),
        ..Default::default()
    });
    let event = sub.receiver.try_recv().unwrap();
    assert_eq!(event.attempt_number, 0);
    assert!(event.scope.is_empty());
    assert!(event.attempt_status.is_empty());
    assert!(event.canonical_model.is_empty());
}

#[test]
fn llm_rich_event_preserves_metrics_and_unrelated_fields_on_both_delivery_paths() {
    let output = Buffer::default();
    let root = ContentStream::new(output.clone());
    root.set_session(9);
    root.set_phase("context phase");
    root.set_step("context step");
    let child = root.child("context task");
    let sub = child.subscribe(1);
    let mut expected = ContentEvent {
        timestamp: serde_json::from_str("\"2020-01-02T03:04:05.123456789+05:30\"").unwrap(),
        kind: "other".into(),
        session: -3,
        phase: "explicit phase".into(),
        step: "explicit step".into(),
        task_id: "explicit task".into(),
        agent_name: "explicit agent".into(),
        attempt_number: i32::MAX,
        scope: "subagent".into(),
        attempt_status: "failed".into(),
        requested_model: "requested".into(),
        resolved_model: "resolved".into(),
        canonical_model: "canonical".into(),
        provider: "provider".into(),
        turn: i32::MIN,
        usage_available: true,
        has_prompt_tokens: true,
        has_completion_tokens: true,
        has_total_tokens: true,
        prompt_tokens: i64::MIN,
        completion_tokens: i64::MAX,
        total_tokens: -7,
        attempt_latency_ms: -1,
        retry_planned: true,
        retry_after_ms: i64::MAX,
        fallback_planned: true,
        fallback_from_model: "from".into(),
        fallback_to_model: "to".into(),
        fallback_reason: " reason\n😀 ".into(),
        failure_kind: "unknown failure".into(),
        message: " message\n".into(),
        output: "x".repeat(40_000),
        input_raw: "y".repeat(40_000),
        subagent_depends_on: vec!["dep".into(), "dep".into()],
        is_error: true,
        ..Default::default()
    };
    child.emit_llm_attempt(expected.clone());
    expected.kind = "llm_attempt".into();
    expected.llm_attempt = i32::MAX;
    expected.llm_scope = "subagent".into();
    expected.status = "failed".into();
    expected.model = "canonical".into();
    let expected_bytes = to_go_json(&expected).unwrap();
    assert_eq!(
        to_go_json(&sub.receiver.try_recv().unwrap()).unwrap(),
        expected_bytes
    );
    let mut expected_wire = expected_bytes;
    expected_wire.push(b'\n');
    assert_eq!(*output.0.lock().unwrap(), expected_wire);
}

#[test]
fn llm_empty_inputs_receive_stream_context_without_inventing_attempt_metrics() {
    let output = Buffer::default();
    let root = ContentStream::new(output.clone());
    root.set_session(7);
    root.set_phase("phase");
    root.set_step("step");
    let child = root.child("task");
    let sub = child.subscribe(2);
    let start: chrono::DateTime<chrono::Utc> = std::time::SystemTime::now().into();
    child.emit_llm_attempt(LlmAttempt::default());
    child.emit_llm_attempt(ContentEvent::default());
    let end: chrono::DateTime<chrono::Utc> = std::time::SystemTime::now().into();
    let mut expected_wire = Vec::new();
    for _ in 0..2 {
        let event = sub.receiver.try_recv().unwrap();
        let timestamp = chrono::DateTime::parse_from_rfc3339(event.timestamp.as_str()).unwrap();
        assert!(timestamp >= start && timestamp <= end);
        assert!(event.timestamp.as_str().ends_with('Z'));
        let expected = ContentEvent {
            timestamp: event.timestamp.clone(),
            kind: "llm_attempt".into(),
            session: 7,
            phase: "phase".into(),
            step: "step".into(),
            task_id: "task".into(),
            ..Default::default()
        };
        let expected_bytes = to_go_json(&expected).unwrap();
        assert_eq!(to_go_json(&event).unwrap(), expected_bytes);
        expected_wire.extend(expected_bytes);
        expected_wire.push(b'\n');
    }
    assert_eq!(*output.0.lock().unwrap(), expected_wire);
}

#[test]
fn llm_inputs_and_one_way_aliases_match_pinned_sdk() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/content-stream/observations.json"
    ))
    .unwrap();
    let cases = fixture["llmCases"].as_array().unwrap();
    assert_eq!(cases.len(), 25);
    for (i, case) in cases.iter().enumerate() {
        let input: ContentEvent = serde_json::from_value(case["input"].clone()).unwrap();
        let start: chrono::DateTime<chrono::Utc> = std::time::SystemTime::now().into();
        let output = super::Buffer::default();
        let root = ContentStream::new(output.clone());
        root.set_session(7);
        root.set_phase("phase");
        root.set_step("step");
        let child = root.child("task");
        let sub = child.subscribe(8);
        if case["form"] == "six" {
            child.emit_llm_attempt(LlmAttempt {
                attempt: input.attempt_number,
                scope: input.scope,
                agent_name: input.agent_name,
                model: input.canonical_model,
                task_id: input.task_id,
                status: input.attempt_status,
            });
        } else {
            child.emit_llm_attempt(input);
        }
        let mut delivered = super::drain(&sub, start);
        delivered["events"] = super::digests(delivered["events"].take());
        assert_eq!(delivered, case["subscriber"], "subscriber case {i}");
        assert_eq!(
            super::digests(super::wire(&output, start)),
            case["wire"],
            "wire case {i}"
        );
    }
}
