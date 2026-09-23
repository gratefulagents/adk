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
fn nonfinite_span_costs_match_pinned_marshal_errors_without_null_records() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(FilesystemTraceStore::new(root.path()).unwrap());
    for (label, cost) in [
        ("nan", f64::NAN),
        ("positive", f64::INFINITY),
        ("negative", f64::NEG_INFINITY),
    ] {
        for (kind, data) in [
            (
                "session",
                SpanData::Session(Session {
                    cost_usd: cost,
                    ..Default::default()
                }),
            ),
            (
                "generation",
                SpanData::Generation(Box::new(Generation {
                    cost_usd: cost,
                    ..Default::default()
                })),
            ),
            (
                "subagent",
                SpanData::Subagent(Box::new(Subagent {
                    cost_usd: cost,
                    ..Default::default()
                })),
            ),
        ] {
            let name = format!("{label}-{kind}");
            let writer = TraceWriter::new(store.clone(), &name, Options::default());
            let path = writer
                .init_run(&RunMetadata {
                    run_id: name.clone(),
                    ..Default::default()
                })
                .unwrap();
            let mut span = Span::new(kind, "", Some(data));
            writer.span_start(&span);
            span.finish();
            writer.span_end(&span);
            assert_eq!(
                serde_json::to_value(writer.health()).unwrap(),
                fixture["nonfinite_span_cases"][&name]
            );
            assert!(!path.join("spans.jsonl").exists());
            assert!(!path.join("llm_calls.jsonl").exists());
        }
    }
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
        spans: vec![],
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
            snapshot_raw: None,
            raw: None,
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
    let output = format!("<>&\u{2028}\u{2029}{}", "uncapped-output ".repeat(30000));
    writer.tool_end("agent", "tool", "call", &output, false, "parent");
    let events = records(&path, "tool_calls");
    assert_eq!(events[0]["input"]["nested"]["password"], "[REDACTED]");
    assert_eq!(events[0]["input"]["nested"]["instruction"], "[OPERATOR]");
    assert_eq!(events[0]["parent_call_id"], "parent");
    assert_eq!(events[1]["output"], output);
    let encoded = fs::read_to_string(path.join("tool_calls.jsonl")).unwrap();
    assert!(encoded.contains(r"\u003c\u003e\u0026\u2028\u2029"));
    assert!(!encoded.contains("<>&"));
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

#[test]
fn explicit_trace_lifecycle_retains_span_order_and_finished_duration() {
    let mut trace = Trace::new("session");
    assert!(uuid::Uuid::parse_str(&trace.id).is_ok());
    assert!(trace.spans.is_empty());
    assert_eq!(trace.end_time.to_rfc3339(), "0001-01-01T00:00:00+00:00");
    let mut first = Span::new("first", &trace.id, None);
    assert!(uuid::Uuid::parse_str(&first.id).is_ok());
    assert_ne!(first.id, trace.id);
    assert_eq!(first.parent_id, trace.id);
    assert_eq!(first.name, "first");
    assert!(first.data.is_none());
    assert!(first.start_time >= trace.start_time);
    assert!(first.duration_ms() >= 0);
    first.finish();
    assert!(first.end_time >= first.start_time);
    let duration = first.duration_ms();
    let second = Span::new("second", &first.id, None);
    trace.add_span(first);
    trace.add_span(second);
    trace.finish();
    assert!(trace.end_time >= trace.start_time);
    assert_eq!(
        trace
            .spans
            .iter()
            .map(|span| span.name.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(trace.spans[0].duration_ms(), duration);
    for (start, end, expected) in [
        ("2025-01-02T03:04:05Z", "2025-01-02T03:04:06.234567Z", 1234),
        ("2025-01-02T03:04:06.234567Z", "2025-01-02T03:04:05Z", -1234),
        ("2025-01-02T03:04:05.000001Z", "2025-01-02T03:04:05Z", 0),
    ] {
        let mut span = Span::new("duration", "", None);
        span.start_time = start.parse().unwrap();
        span.end_time = end.parse().unwrap();
        assert_eq!(span.duration_ms(), expected);
    }
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    for case in fixture["duration_cases"].as_array().unwrap() {
        let mut span = Span::new("duration", "", None);
        span.start_time = case["start"].as_str().unwrap().parse().unwrap();
        span.end_time = case["end"].as_str().unwrap().parse().unwrap();
        assert_eq!(span.duration_ms(), case["duration_ms"].as_i64().unwrap());
    }
}

#[tokio::test]
async fn actual_runner_writes_ordered_generation_attempts_without_otel() {
    use adk::core::*;
    use adk::runtime::{
        AgentConfig, CancellationToken, ModelBinding, RetryPolicy, Runner, RunnerConfig,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Provider(AtomicUsize, u64);
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
                if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Err(Error::new(ErrorCategory::Provider, "retry"));
                }
                Ok(ModelResponse {
                    snapshot_raw: None,
                    raw: Some(json!({"provider_extra": "retained"})),
                    items: vec![RunItem::Message {
                        message: Message {
                            role: Role::Assistant,
                            content: vec![Content::Text {
                                text: "done".into(),
                            }],
                        },
                    }],
                    usage: Usage {
                        requests: self.1,
                        input_tokens: 12,
                        output_tokens: 3,
                        ..Default::default()
                    },
                    end_turn: Some(true),
                    response_id: None,
                    metadata: Default::default(),
                })
            })
        }
    }
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
    for (capture, requests) in [
        (CaptureMode::Metadata, 1),
        (CaptureMode::Full, 1),
        (CaptureMode::Full, u64::MAX),
    ] {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(FilesystemTraceStore::new(root.path()).unwrap());
        let writer = Arc::new(TraceWriter::new(
            store,
            "run",
            Options {
                capture,
                ..Default::default()
            },
        ));
        let path = writer
            .init_run(&RunMetadata {
                run_id: "run".into(),
                ..Default::default()
            })
            .unwrap();
        let runner = Runner::new(
            AgentConfig::new(
                "agent",
                ModelBinding::complete("model", Arc::new(Provider(AtomicUsize::new(0), requests))),
            ),
            RunnerConfig {
                generation_observer: Some(writer.clone()),
                retry: RetryPolicy {
                    max_retries: 1,
                    initial_delay: std::time::Duration::ZERO,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        let result = runner
            .run(
                Context {
                    run_id: "run".into(),
                    cancellation: Arc::new(CancellationToken::new()),
                    deadline: None,
                },
                RunRequest {
                    input_provenance: Vec::new(),
                    input: vec![],
                    policy: RunPolicy::default(),
                },
                Arc::new(NoopHost),
            )
            .await
            .unwrap();
        assert_eq!(result.result.final_output, Some(json!("done")));
        let spans = records(&path, "spans");
        let calls = records(&path, "llm_calls");
        assert_eq!(spans.as_array().unwrap().len(), 4);
        assert_eq!(calls.as_array().unwrap().len(), 4);
        for (index, kind) in [
            "generation_start",
            "generation_end",
            "generation_start",
            "generation_end",
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(calls[index]["type"], *kind);
        }
        assert_eq!(calls[1]["status"], "retrying");
        assert_eq!(calls[1]["retry_scheduled"], true);
        assert_eq!(calls[3]["status"], "completed");
        assert_eq!(calls[3]["input_tokens"], 12);
        assert_eq!(calls[3]["output_tokens"], 3);
        assert_eq!(calls[3]["total_tokens"], 15);
        use sha2::{Digest, Sha256};
        let fixture: Value =
            serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json"))
                .unwrap();
        let bytes = fixture["automatic_response_json"]
            .as_str()
            .unwrap()
            .as_bytes();
        if requests == u64::MAX {
            assert!(calls[3].get("response").is_none());
            assert!(
                writer
                    .health()
                    .last_error
                    .contains("usage counter exceeds SDK signed range")
            );
            writer.finalize_run("completed").unwrap();
            let health: Value =
                serde_json::from_slice(&fs::read(path.join("trace_health.json")).unwrap()).unwrap();
            assert_eq!(health["last_error"], writer.health().last_error);
        } else if capture == CaptureMode::Metadata {
            assert_eq!(
                calls[3]["response"],
                json!({"captured": false, "sha256": format!("{:x}", Sha256::digest(bytes)), "bytes": bytes.len()})
            );
        } else {
            assert_eq!(
                calls[3]["response"],
                serde_json::from_slice::<Value>(bytes).unwrap()
            );
        }
        assert!(calls[1].get("response").is_none());
        assert_eq!(writer.health().write_errors, 0);
        if requests != u64::MAX {
            assert!(writer.health().last_error.is_empty());
        }
    }
}

#[test]
fn typed_request_metadata_digests_match_pinned_go_serialized_bytes() {
    use sha2::{Digest, Sha256};
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(FilesystemTraceStore::new(root.path()).unwrap());
    let writer = TraceWriter::new(store, "run", Options::default());
    let path = writer
        .init_run(&RunMetadata {
            run_id: "run".into(),
            ..Default::default()
        })
        .unwrap();
    let variants = fixture["request_variants"].as_array().unwrap();
    for variant in variants {
        let request: adk::codec::snapshots::RequestSnapshot =
            serde_json::from_str(variant.as_str().unwrap()).unwrap();
        let span = Span::new(
            "generation",
            "run",
            Some(SpanData::Generation(Box::new(Generation {
                request: Some(Snapshot::from_serializable(&request).unwrap()),
                ..Default::default()
            }))),
        );
        writer.span_start(&span);
    }
    let calls = records(&path, "llm_calls");
    for (index, variant) in variants.iter().enumerate() {
        let expected = variant.as_str().unwrap().as_bytes();
        assert_eq!(
            calls[index]["request"],
            json!({ "captured": false, "sha256": format!("{:x}", Sha256::digest(expected)), "bytes": expected.len() })
        );
    }
}

#[test]
fn runtime_generation_identity_matches_pinned_go_normalization() {
    use adk::core::{Context, ModelRequest};
    use adk::runtime::{
        CancellationToken,
        tracing::{GenerationObserver, GenerationRecord, GenerationStatus},
    };
    use std::time::{Duration, SystemTime};
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(FilesystemTraceStore::new(root.path()).unwrap());
    let writer = TraceWriter::new(store, "run", Options::default());
    let path = writer
        .init_run(&RunMetadata {
            run_id: "run".into(),
            ..Default::default()
        })
        .unwrap();
    let context = Context {
        run_id: "run".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    };
    let cases = fixture["model_identities"].as_array().unwrap();
    for case in cases {
        writer.start(
            &context,
            &GenerationRecord {
                id: "generation".into(),
                agent: "agent".into(),
                provider: case["provider"].as_str().unwrap().into(),
                resolved_model: "".into(),
                input_tokens_include_cache: None,
                task_id: None,
                cost_usd: None,
                turn: 1,
                declared_tool_timeouts: Vec::new(),
                request_snapshot: Err(adk_codec::approval::BridgeError(
                    "synthetic identity-only record",
                )),
                request: ModelRequest {
                    input_provenance: Vec::new(),
                    model: case["raw"].as_str().unwrap().into(),
                    instructions: "".into(),
                    input: vec![],
                    tools: vec![],
                    output_schema: None,
                    output_schema_name: "".into(),
                    output_schema_strict: false,
                    settings: Default::default(),
                },
                response: None,
                error: None,
                retry_reason: None,
                status: GenerationStatus::Started,
                retry_after: None,
                fallback_model: None,
                started_at: SystemTime::now(),
                ended_at: None,
                latency: Duration::ZERO,
            },
        );
    }
    let calls = records(&path, "llm_calls");
    assert_eq!(calls.as_array().unwrap().len(), cases.len());
    for (call, case) in calls.as_array().unwrap().iter().zip(cases) {
        assert_eq!(call["requested_model"], case["identity"]["Raw"], "{case}");
        assert_eq!(
            call["model_provider"], case["identity"]["Provider"],
            "{case}"
        );
        assert_eq!(
            call["model_canonical"], case["identity"]["Canonical"],
            "{case}"
        );
    }
}

#[test]
fn captured_attempt_request_and_conversion_errors_reach_writer() {
    use adk::core::{Context, ModelRequest};
    use adk::runtime::{
        CancellationToken,
        tracing::{GenerationObserver, GenerationRecord, GenerationStatus},
    };
    use adk_codec::snapshots::RequestSnapshot;
    use std::time::{Duration, SystemTime};
    let root = tempfile::tempdir().unwrap();
    let writer = TraceWriter::new(
        Arc::new(FilesystemTraceStore::new(root.path()).unwrap()),
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
    let context = Context {
        run_id: "run".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    };
    let request = ModelRequest {
        model: "model".into(),
        instructions: "instruction".into(),
        input: vec![],
        input_provenance: vec![],
        tools: vec![],
        settings: Default::default(),
        output_schema: None,
        output_schema_name: String::new(),
        output_schema_strict: false,
    };
    let snapshot = RequestSnapshot::from_native("agent", &request, &[]).unwrap();
    let mut record = GenerationRecord {
        id: "attempt".into(),
        agent: "agent".into(),
        provider: "fixture".into(),
        resolved_model: "model".into(),
        input_tokens_include_cache: None,
        task_id: None,
        cost_usd: None,
        turn: 1,
        request,
        declared_tool_timeouts: Vec::new(),
        request_snapshot: Ok(snapshot.clone()),
        response: None,
        error: None,
        retry_reason: None,
        status: GenerationStatus::Completed,
        retry_after: None,
        fallback_model: None,
        started_at: SystemTime::now(),
        ended_at: Some(SystemTime::now()),
        latency: Duration::ZERO,
    };
    writer.end(&context, &record);
    let calls = records(&path, "llm_calls");
    assert_eq!(
        calls[0]["request"],
        serde_json::to_value(&snapshot).unwrap()
    );
    assert_eq!(
        calls[0]["input_token_estimate"],
        snapshot.input_token_estimate
    );
    assert_eq!(
        calls[0]["request_overhead_token_estimate"],
        snapshot.request_overhead_token_estimate
    );
    assert_eq!(
        calls[0]["total_request_token_estimate"],
        snapshot.total_token_estimate
    );
    record.request_snapshot = Err(adk_codec::approval::BridgeError(
        "unknown historical author",
    ));
    writer.end(&context, &record);
    assert!(writer.health().last_error.contains(
        "snapshot request: unsupported approval/history conversion: unknown historical author"
    ));
    let calls = records(&path, "llm_calls");
    assert!(calls[1].get("request").is_none());
}
