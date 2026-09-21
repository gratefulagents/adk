#![cfg(all(feature = "observability", target_os = "linux"))]
use adk::{
    core::{Content, Message, ModelResponse, Role, RunItem, Usage},
    observability::CaptureMode,
    tracestore::{FilesystemTraceStore, Limits, RunMetadata},
    tracewriter::*,
};
use serde_json::{Value, json};
use std::{fs, sync::Arc};

fn normalize(mut value: Value) -> Value {
    match &mut value {
        Value::Object(fields) => {
            for key in ["timestamp", "start_time", "end_time"] {
                fields.remove(key);
            }
            if fields
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "tool_end")
            {
                fields.remove("duration_ms");
            }
            for value in fields.values_mut() {
                *value = normalize(value.take());
            }
        }
        Value::Array(items) => {
            for value in items {
                *value = normalize(value.take());
            }
        }
        Value::Number(number) if number.is_f64() => {
            let number = number.as_f64().unwrap();
            if number.fract() == 0.0 && number.abs() < 9007199254740992.0 {
                value = json!(number as i64);
            }
        }
        _ => {}
    }
    value
}
fn records(path: &std::path::Path, category: &str) -> Value {
    Value::Array(
        fs::read_to_string(path.join(format!("{category}.jsonl")))
            .unwrap()
            .lines()
            .map(|line| normalize(serde_json::from_str(line).unwrap()))
            .collect(),
    )
}
#[test]
fn all_span_kinds_and_hook_categories_match_independent_pinned_go() {
    let expected: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(FilesystemTraceStore::new(root.path()).unwrap());
    let writer = TraceWriter::new(store.clone(), "run", Options::default());
    let start = "2025-01-02T03:04:05Z".parse().unwrap();
    let end = "2025-01-02T03:04:06Z".parse().unwrap();
    let path = writer
        .init_run(&RunMetadata {
            run_id: "run".into(),
            started_at: start,
            ..Default::default()
        })
        .unwrap();
    let request = Snapshot::from_json(
        expected["request_json"]
            .as_str()
            .unwrap()
            .as_bytes()
            .to_vec(),
    )
    .unwrap();
    let response = Snapshot::from_json(
        expected["response_json"]
            .as_str()
            .unwrap()
            .as_bytes()
            .to_vec(),
    )
    .unwrap();
    let data = [
        SpanData::Generation(Box::new(Generation {
            requested_model: "requested".into(),
            resolved_model: "resolved".into(),
            model_provider: "provider".into(),
            model_canonical: "canonical".into(),
            attempt_number: 2,
            generation_turn: 3,
            scope: "root".into(),
            task_id: "task".into(),
            status: "completed".into(),
            usage_available: true,
            prompt_tokens: 12,
            completion_tokens: 3,
            cache_read_tokens: 4,
            cache_creation_tokens: 2,
            total_tokens: 15,
            cost_usd: 0.5,
            cost_known: true,
            latency_ms: 1000,
            success: true,
            tool_count: 1,
            input_item_count: 2,
            output_item_count: 1,
            instructions_length: 19,
            input_token_estimate: 8,
            request_overhead_token_estimate: 4,
            total_request_token_estimate: 12,
            request: Some(request),
            response: Some(response),
            ..Default::default()
        })),
        SpanData::Function {
            input: String::new(),
            output: String::new(),
            tool_name: "Read".into(),
            is_error: true,
        },
        SpanData::Handoff {
            from_agent: "a".into(),
            to_agent: "b".into(),
        },
        SpanData::Guardrail {
            guardrail_name: "guard".into(),
            triggered: true,
        },
        SpanData::Compaction {
            tokens_before: 100,
            tokens_after: 50,
        },
        SpanData::Session(Session {
            model: "model".into(),
            cost_usd: 0.5,
            num_turns: 2,
            duration_ms: 900,
            input_tokens: 12,
            output_tokens: 3,
            cache_read_input_tokens: 4,
            cache_creation_input_tokens: 2,
            stop_reason: "completed".into(),
        }),
        SpanData::Subagent(Box::new(Subagent {
            task_id: "child".into(),
            subagent_type: "researcher".into(),
            description: "task description".into(),
            model: "model".into(),
            status: "completed".into(),
            cost_usd: 0.5,
            num_turns: 2,
            total_tokens: 15,
            input_tokens: 12,
            output_tokens: 3,
            cache_read_tokens: 4,
            cache_creation_tokens: 2,
            tool_count: 1,
            duration_ms: 900,
            stop_reason: "completed".into(),
            isolation: "worktree".into(),
            prompt: "child prompt".into(),
            result_text: "child result".into(),
            files_read: vec!["input.rs".into()],
            files_written: vec!["output.rs".into()],
        })),
        SpanData::Agent {
            instructions: String::new(),
            agent_name: "agent".into(),
        },
        SpanData::Retry {
            retry_after_ms: 0,
            max_retries: 0,
            error_code: "rate_limit".into(),
            attempt: 2,
        },
    ];
    let trace = Trace {
        id: "trace".into(),
        name: "fixture".into(),
        start_time: start,
        end_time: end,
    };
    writer.trace_start(&trace);
    for data in data {
        let span = Span {
            id: "span".into(),
            parent_id: "trace".into(),
            name: "operation".into(),
            start_time: start,
            end_time: end,
            data: Some(data),
        };
        writer.span_start(&span);
        writer.span_end(&span);
    }
    writer.trace_end(&trace);
    writer.agent_start("agent");
    writer.llm_start("agent", "model");
    writer.llm_end(
        "agent",
        &ModelResponse {
            items: vec![RunItem::Message {
                message: Message {
                    role: Role::Assistant,
                    content: vec![Content::Text {
                        text: "answer".into(),
                    }],
                },
            }],
            usage: Usage {
                input_tokens: 12,
                output_tokens: 3,
                ..Default::default()
            },
            end_turn: None,
            response_id: None,
            metadata: Default::default(),
        },
    );
    writer.tool_start("agent", "Bash", "call", br#"{"command":"pwd"}"#, "");
    writer.tool_end("agent", "Bash", "call", "tool output", false, "");
    writer.handoff("agent", "specialist");
    writer.agent_end("agent");
    writer.record_phase_change("verify");
    writer.record_mode_switch("code", "plan").unwrap();
    writer.write_resolved_instructions(4, "other instructions");
    writer
        .write_metrics(json!({"turns":2}).as_object().unwrap())
        .unwrap();
    writer.finalize_run("completed").unwrap();
    for category in ["spans", "llm_calls", "tool_calls", "agent_transitions"] {
        assert_eq!(records(&path, category), expected[category], "{category}");
    }
    for name in [
        "resolved_instructions/turn_003_attempt_002.txt",
        "resolved_instructions/turn_004.txt",
        "trace_health.json",
        "metrics.json",
    ] {
        let actual: Value = serde_json::from_slice(&fs::read(path.join(name)).unwrap()).unwrap();
        assert_eq!(normalize(actual), expected[name], "{name}");
    }
}

#[test]
fn full_capture_preserves_large_raw_output_and_recursively_redacts_credentials() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(FilesystemTraceStore::new(root.path()).unwrap());
    let writer = TraceWriter::new(
        store,
        "run",
        Options {
            capture: CaptureMode::Full,
            redactors: vec![Arc::new(|text| {
                text.replace("operator-private", "[OPERATOR]")
            })],
        },
    );
    let path = writer
        .init_run(&RunMetadata {
            run_id: "run".into(),
            ..Default::default()
        })
        .unwrap();
    let input =
        json!({"nested":{"password":"fixture-placeholder", "instruction":"operator-private"}});
    writer.tool_start(
        "agent",
        "tool",
        "call",
        &serde_json::to_vec(&input).unwrap(),
        "parent",
    );
    let output = "uncapped-output ".repeat(30000);
    writer.tool_end("agent", "tool", "call", &output, false, "parent");
    let events = records(&path, "tool_calls");
    assert_eq!(events[0]["input"]["nested"]["password"], "[REDACTED]");
    assert_eq!(events[0]["input"]["nested"]["instruction"], "[OPERATOR]");
    assert_eq!(events[0]["parent_call_id"], "parent");
    assert_eq!(events[1]["output"], output);
    assert_eq!(events[1]["parent_call_id"], "parent");
    assert_eq!(writer.health().events_written, 2);
}

#[test]
fn truncation_and_store_quota_failures_are_visible_in_health() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(FilesystemTraceStore::new(root.path()).unwrap());
    let writer = TraceWriter::new(
        store,
        "run",
        Options {
            capture: CaptureMode::Full,
            ..Default::default()
        },
    );
    let path = writer
        .init_run(&RunMetadata {
            run_id: "run".into(),
            ..Default::default()
        })
        .unwrap();
    writer.tool_end("agent", "tool", "call", &"x".repeat(1 << 20), false, "");
    assert_eq!(records(&path, "tool_calls")[0]["type"], "event_truncated");
    assert_eq!(writer.health().events_truncated, 1);
    writer.finalize_run("completed").unwrap();
    let health: Value =
        serde_json::from_slice(&fs::read(path.join("trace_health.json")).unwrap()).unwrap();
    assert_eq!(health["events_truncated"], 1);

    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(
        FilesystemTraceStore::with_limits(
            root.path(),
            Limits {
                event_bytes: 16,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let writer = TraceWriter::new(store, "run", Options::default());
    writer
        .init_run(&RunMetadata {
            run_id: "run".into(),
            ..Default::default()
        })
        .unwrap();
    writer.agent_start("agent");
    assert_eq!(writer.health().events_dropped, 1);
    assert_eq!(writer.health().write_errors, 0);
    assert!(!writer.health().last_error.is_empty());
}
