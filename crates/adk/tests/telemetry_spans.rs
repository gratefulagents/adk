#![cfg(feature = "otel")]
use adk::{
    telemetry::Telemetry,
    tracewriter::{Generation, Session, Span, SpanData, Subagent, Trace},
};
use opentelemetry_sdk::trace::InMemorySpanExporter;
use serde_json::Value;
use std::sync::{Arc, Mutex};

fn trace() -> Trace {
    Trace {
        spans: vec![],
        id: "trace".into(),
        name: "fixture".into(),
        start_time: "2025-01-02T03:04:05Z".parse().unwrap(),
        end_time: "2025-01-02T03:04:06Z".parse().unwrap(),
    }
}
fn span(id: &str, parent: &str, data: SpanData) -> Span {
    let t = trace();
    Span {
        id: id.into(),
        parent_id: parent.into(),
        name: "operation".into(),
        start_time: t.start_time,
        end_time: t.end_time,
        data: Some(data),
    }
}
fn spans() -> Vec<SpanData> {
    vec![
        SpanData::Agent {
            agent_name: "agent".into(),
            instructions: "instruction".into(),
        },
        SpanData::Generation(Box::new(Generation {
            requested_model: "requested".into(),
            resolved_model: "resolved".into(),
            attempt_number: 2,
            generation_turn: 3,
            status: "completed".into(),
            usage_available: true,
            prompt_tokens: 12,
            completion_tokens: 3,
            total_tokens: 15,
            success: true,
            cost_usd: 0.5,
            input_tokens_include_cache: true,
            input_tokens_include_cache_known: true,
            ..Default::default()
        })),
        SpanData::Function {
            tool_name: "Read".into(),
            input: "input".into(),
            output: "failure".into(),
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
            ..Default::default()
        })),
        SpanData::Subagent(Box::new(Subagent {
            subagent_type: "executor".into(),
            cache_read_tokens: 1200,
            cache_creation_tokens: 300,
            ..Default::default()
        })),
        SpanData::Retry {
            error_code: "rate_limit".into(),
            attempt: 2,
            retry_after_ms: 500,
            max_retries: 3,
        },
    ]
}

#[test]
fn cost_attributes_preserve_ieee_floats_without_json_conversion() {
    let exporter = InMemorySpanExporter::default();
    let telemetry = Telemetry::with_exporter("fixture", exporter.clone());
    let processor = telemetry.span_processor();
    processor.on_trace_start(&trace());
    let values = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.0, f64::MAX];
    for cost in values {
        for data in [
            SpanData::Session(Session {
                cost_usd: cost,
                ..Default::default()
            }),
            SpanData::Generation(Box::new(Generation {
                cost_usd: cost,
                ..Default::default()
            })),
            SpanData::Subagent(Box::new(Subagent {
                cost_usd: cost,
                ..Default::default()
            })),
        ] {
            let span = span("span", "trace", data);
            processor.on_span_start(&span);
            processor.on_span_end(&span);
        }
    }
    processor.on_trace_end(&trace());
    telemetry.force_flush().unwrap();
    let exported = exporter.get_finished_spans().unwrap();
    assert_eq!(exported.len(), values.len() * 3 + 1);
    let spans: Vec<_> = exported
        .iter()
        .filter(|span| {
            span.attributes
                .iter()
                .any(|attr| attr.key.as_str().ends_with(".cost_usd"))
        })
        .collect();
    assert_eq!(spans.len(), values.len() * 3);
    for (chunk, expected) in spans.chunks(3).zip(values) {
        for (span, key) in
            chunk
                .iter()
                .zip(["session.cost_usd", "gen.cost_usd", "subagent.cost_usd"])
        {
            let value = &span
                .attributes
                .iter()
                .find(|attr| attr.key.as_str() == key)
                .unwrap()
                .value;
            let opentelemetry::Value::F64(actual) = value else {
                panic!("not an f64: {value:?}")
            };
            if expected.is_nan() {
                assert!(actual.is_nan());
            } else {
                assert_eq!(actual.to_bits(), expected.to_bits());
            }
        }
    }
}

#[test]
fn every_span_kind_matches_independent_pinned_go_export() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-otel.json")).unwrap();
    let exporter = InMemorySpanExporter::default();
    let telemetry = Telemetry::with_exporter("fixture", exporter.clone());
    let processor = telemetry.span_processor();
    processor.on_trace_start(&trace());
    for data in spans() {
        let s = span("span", "trace", data);
        processor.on_span_start(&s);
        processor.on_span_end(&s);
    }
    processor.on_trace_end(&trace());
    telemetry.force_flush().unwrap();
    let exported = exporter.get_finished_spans().unwrap();
    assert_eq!(exported.len(), fixture.as_object().unwrap().len());
    for s in exported {
        let expected = &fixture[s.name.as_ref()];
        let attrs = expected["attributes"].as_object().unwrap();
        assert_eq!(s.attributes.len(), attrs.len(), "{}", s.name);
        for attr in &s.attributes {
            let expected = &attrs[attr.key.as_str()];
            match &attr.value {
                opentelemetry::Value::Bool(v) => {
                    assert_eq!(Some(*v), expected.as_bool(), "{}", attr.key)
                }
                opentelemetry::Value::I64(v) => {
                    assert_eq!(Some(*v), expected.as_i64(), "{}", attr.key)
                }
                opentelemetry::Value::F64(v) => {
                    assert_eq!(Some(*v), expected.as_f64(), "{}", attr.key)
                }
                opentelemetry::Value::String(v) => {
                    assert_eq!(Some(v.as_str()), expected.as_str(), "{}", attr.key)
                }
                _ => panic!("unexpected array"),
            }
        }
        match s.status {
            opentelemetry::trace::Status::Error { description } => {
                assert_eq!(expected["status"]["Code"], "Error");
                assert_eq!(expected["status"]["Description"], description.as_ref());
            }
            _ => assert_eq!(expected["status"]["Code"], "Unset"),
        }
    }
    telemetry.shutdown().unwrap();
}

#[test]
fn trace_id_callback_is_reentrant_once_and_ended_parents_remain_available() {
    let exporter = InMemorySpanExporter::default();
    let telemetry = Telemetry::with_exporter("fixture", exporter.clone());
    let processor = telemetry.span_processor();
    assert_eq!(processor.trace_id(), "");
    let seen = Arc::new(Mutex::new(vec![]));
    let weak = Arc::downgrade(&processor);
    let seen_callback = seen.clone();
    processor.set_on_trace_id_ready(Some(Arc::new(move |id| {
        assert_eq!(weak.upgrade().unwrap().trace_id(), id);
        seen_callback.lock().unwrap().push(id);
    })));
    processor.on_trace_start(&trace());
    let root_id = processor.trace_id();
    let parent = span(
        "parent",
        "trace",
        SpanData::Agent {
            agent_name: "parent".into(),
            instructions: String::new(),
        },
    );
    processor.on_span_start(&parent);
    processor.on_span_end(&parent);
    let child = span(
        "child",
        "parent",
        SpanData::Agent {
            agent_name: "child".into(),
            instructions: String::new(),
        },
    );
    processor.on_span_start(&child);
    processor.on_span_end(&child);
    processor.on_trace_end(&trace());
    let late = span(
        "late",
        "unknown",
        SpanData::Agent {
            agent_name: "late".into(),
            instructions: String::new(),
        },
    );
    processor.on_span_start(&late);
    processor.on_span_end(&late);
    processor.on_trace_start(&trace());
    processor.on_trace_end(&trace());
    telemetry.force_flush().unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![root_id]);
    let exported = exporter.get_finished_spans().unwrap();
    let parent = exported.iter().find(|s| s.name == "agent.parent").unwrap();
    let child = exported.iter().find(|s| s.name == "agent.child").unwrap();
    let late = exported.iter().find(|s| s.name == "agent.late").unwrap();
    assert_eq!(child.parent_span_id, parent.span_context.span_id());
    assert_eq!(late.span_context.trace_id(), parent.span_context.trace_id());
    telemetry.shutdown().unwrap();
}

#[test]
fn final_attributes_replace_start_values_and_sensitive_text_is_redacted() {
    let exporter = InMemorySpanExporter::default();
    let telemetry = Telemetry::with_exporter("fixture", exporter.clone());
    let processor = telemetry.span_processor();
    processor.on_trace_start(&trace());
    let mut s = span(
        "tool",
        "trace",
        SpanData::Function {
            tool_name: "test".into(),
            input: "Bearer fake-token".into(),
            output: "pending".into(),
            is_error: false,
        },
    );
    processor.on_span_start(&s);
    s.data = Some(SpanData::Function {
        tool_name: "test".into(),
        input: "Bearer fake-token".into(),
        output: "Bearer another-token".into(),
        is_error: true,
    });
    processor.on_span_end(&s);
    processor.on_span_end(&s);
    processor.on_trace_end(&trace());
    telemetry.force_flush().unwrap();
    let exported = exporter.get_finished_spans().unwrap();
    let tool = exported.iter().find(|s| s.name == "tool.test").unwrap();
    assert_eq!(exported.len(), 2);
    let rendered = format!("{tool:?}");
    assert!(
        !rendered.contains("fake-token")
            && !rendered.contains("another-token")
            && !rendered.contains("pending")
    );
    assert!(rendered.contains("[REDACTED]"));
    assert!(matches!(
        tool.status,
        opentelemetry::trace::Status::Error { .. }
    ));
    telemetry.shutdown().unwrap();
}

#[tokio::test]
async fn actual_runner_exports_generation_retry_usage_identity_and_single_cost_estimate() {
    use adk_core::*;
    use adk_runtime::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct ModelImpl(AtomicUsize);
    impl Model for ModelImpl {
        fn provider(&self) -> &str {
            "router"
        }
        fn info(&self, _: &str) -> ModelInfo {
            ModelInfo {
                provider: "test-wire".into(),
                model: "wire-model".into(),
                input_tokens_include_cache: Some(true),
            }
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
                    raw: None,
                    items: vec![RunItem::Message {
                        message: Message {
                            role: Role::Assistant,
                            content: vec![Content::Text {
                                text: "done".into(),
                            }],
                        },
                    }],
                    usage: Usage {
                        input_tokens: 12,
                        output_tokens: 3,
                        cache_read_tokens: 4,
                        context_tokens: Some(12),
                        ..Default::default()
                    },
                    end_turn: None,
                    response_id: None,
                    metadata: Default::default(),
                })
            })
        }
    }
    struct HostImpl;
    impl Host for HostImpl {
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
    struct Cost(AtomicUsize);
    impl CostEstimator for Cost {
        fn cost(&self, _: &str, _: &Usage) -> f64 {
            self.0.fetch_add(1, Ordering::SeqCst);
            0.5
        }
    }
    let exporter = InMemorySpanExporter::default();
    let telemetry = Telemetry::with_exporter("fixture", exporter.clone());
    let processor = telemetry.span_processor();
    processor.on_trace_start(&trace());
    let cost = Arc::new(Cost(AtomicUsize::new(0)));
    let runner = Runner::new(
        AgentConfig::new(
            "agent",
            ModelBinding::complete("route/alias", Arc::new(ModelImpl(AtomicUsize::new(0)))),
        ),
        RunnerConfig {
            generation_observer: Some(processor.clone()),
            cost_estimator: Some(cost.clone()),
            retry: RetryPolicy {
                max_retries: 1,
                initial_delay: std::time::Duration::ZERO,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    let context = Context {
        run_id: "trace".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    };
    let result = runner
        .run(
            context,
            RunRequest {
                input_provenance: Vec::new(),
                input: vec![],
                policy: RunPolicy::default(),
            },
            Arc::new(HostImpl),
        )
        .await
        .unwrap();
    assert_eq!(result.result.final_output, Some(serde_json::json!("done")));
    assert_eq!(cost.0.load(Ordering::SeqCst), 1);
    processor.on_trace_end(&trace());
    telemetry.force_flush().unwrap();
    let exported = exporter.get_finished_spans().unwrap();
    let generations = exported
        .iter()
        .filter(|s| s.name == "llm.generation")
        .collect::<Vec<_>>();
    assert_eq!(generations.len(), 2);
    let attrs = |span: &opentelemetry_sdk::trace::SpanData| {
        span.attributes
            .iter()
            .map(|attr| (attr.key.as_str().to_owned(), attr.value.to_string()))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let failed = attrs(generations[0]);
    assert_eq!(failed["gen.status"], "retrying");
    assert_eq!(failed["gen.retry_scheduled"], "true");
    let succeeded = attrs(generations[1]);
    for (key, value) in [
        ("gen.status", "completed"),
        ("gen.requested_model", "route/alias"),
        ("gen.resolved_model", "wire-model"),
        ("gen.model_provider", "test-wire"),
        ("gen.model_canonical", "route/alias"),
        ("gen.scope", "top_level"),
        ("gen.input_tokens", "12"),
        ("gen.output_tokens", "3"),
        ("gen.total_tokens", "15"),
        ("gen.cost_usd", "0.5"),
        ("gen.cost_known", "true"),
        ("gen.input_tokens_include_cache", "true"),
        ("gen.input_tokens_include_cache_known", "true"),
    ] {
        assert_eq!(succeeded[key], value, "{key}");
    }
    telemetry.shutdown().unwrap();
}
