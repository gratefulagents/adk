#![cfg(feature = "observability")]

use adk::{
    core::*,
    observability::*,
    runtime::{CancellationToken, Observation, RunHooks},
};
use serde_json::json;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn context() -> Context {
    Context {
        run_id: "run-1".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn record(sequence: u64) -> EventRecord {
    EventRecord {
        schema_version: EVENT_SCHEMA_VERSION,
        run_id: "run-1".into(),
        sequence,
        timestamp_unix_ms: 1000,
        kind: "log".into(),
        data: json!({"text":"héllo\nworld"}),
        progress: ProgressSnapshot::default(),
    }
}
#[derive(Default)]
struct Records(Mutex<Vec<EventRecord>>);
impl EventSink for Records {
    fn emit<'a>(&'a self, record: &'a EventRecord) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(record.clone());
            Ok(())
        })
    }
}
fn pipeline(mode: CaptureMode) -> (Arc<Observability>, Arc<Records>) {
    let records = Arc::new(Records::default());
    let pipeline = Arc::new(
        Observability::new(
            "run-1",
            CapturePolicy {
                mode,
                redactors: vec![],
            },
            vec![records.clone()],
        )
        .unwrap(),
    );
    (pipeline, records)
}
fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "test".into(),
        arguments: json!({"command":"private input"}),
    }
}
fn output(text: &str) -> ToolOutput {
    ToolOutput {
        content: vec![Content::Text { text: text.into() }],
        is_error: false,
        should_pause: false,
    }
}

#[test]
fn jsonl_fragmentation_multiple_lines_unicode_and_eof() {
    let first = record(1);
    let second = record(2);
    let mut bytes = first.to_json_line().unwrap();
    bytes.extend(b"\n \t\r\n");
    bytes.extend(second.to_json_line().unwrap());
    for width in [1, 2, 3, 11, bytes.len()] {
        let mut decoder = LineDecoder::new(DEFAULT_MAX_EVENT_BYTES).unwrap();
        let mut result = Vec::new();
        for chunk in bytes.chunks(width) {
            result.extend(decoder.push(chunk).into_iter().map(Result::unwrap));
        }
        assert_eq!(result, vec![first.clone(), second.clone()]);
        assert!(decoder.finish().is_none());
    }
    let mut decoder = LineDecoder::new(DEFAULT_MAX_EVENT_BYTES).unwrap();
    let last = first.to_json_line().unwrap();
    assert!(decoder.push(&last[..last.len() - 1]).is_empty());
    assert_eq!(decoder.finish().unwrap().unwrap(), first);
    assert!(decoder.finish().is_none());
}

#[test]
fn jsonl_bad_and_oversized_lines_recover_at_delimiters() {
    let valid = record(1).to_json_line().unwrap();
    let mut decoder = LineDecoder::new(valid.len()).unwrap();
    assert!(decoder.push(b"not json\n")[0].is_err());
    assert!(decoder.push(&vec![b'x'; valid.len() + 1])[0].is_err());
    assert!(decoder.push(b"still oversized").is_empty());
    assert!(decoder.push(b"\n").is_empty());
    assert_eq!(decoder.push(&valid)[0].as_ref().unwrap(), &record(1));
    assert!(LineDecoder::new(0).is_err());
    let mut unsupported = record(1);
    unsupported.schema_version += 1;
    assert!(EventRecord::from_json_line(&unsupported.to_json_line().unwrap()).is_err());
    assert!(EventRecord::from_json_line(&record(0).to_json_line().unwrap()).is_err());
    assert!(EventRecord::from_json_line(b"{\"kind\":").is_err());
}

#[test]
fn metadata_defaults_to_exact_digests_without_content() {
    let captured = CapturePolicy::default().capture(&json!({"output":"hello","input":{"password":"plain"},"model":"model-1","usage":{"input_tokens":5},"mystery":"sensitive"}));
    assert_eq!(
        captured["output"],
        json!({"sha256":"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824","bytes":5})
    );
    assert_eq!(captured["model"], "model-1");
    assert_eq!(captured["usage"]["input_tokens"], 5);
    assert!(captured["input"]["sha256"].is_string());
    assert!(captured["mystery"]["sha256"].is_string());
    assert!(!captured.to_string().contains("sensitive"));
    assert!(!captured.to_string().contains("plain"));
}

#[test]
fn full_capture_redacts_nested_keys_json_strings_credentials_and_custom_content() {
    let policy = CapturePolicy {
        mode: CaptureMode::Full,
        redactors: vec![Arc::new(|text| {
            text.replace("project-private", "[OPERATOR]")
        })],
    };
    let token = format!("sk-{}", "a".repeat(48));
    let result = policy.capture(&json!({"output":[{"password":"short","token":"short","text":token},{"text":"{\"api_key\":\"secret value\",\"ok\":\"visible\"}"}, {"text":"project-private and safe"}]}));
    assert_eq!(result["output"][0]["password"], "[REDACTED]");
    assert_eq!(result["output"][0]["token"], "[REDACTED]");
    assert_eq!(result["output"][0]["text"], "[REDACTED]");
    assert!(!result.to_string().contains("secret value"));
    assert!(!result.to_string().contains(&token));
    assert!(result.to_string().contains("[OPERATOR] and safe"));
    assert!(result.to_string().contains("visible"));
}

#[tokio::test]
async fn agent_start_events_do_not_expand_capture_to_instructions() {
    for mode in [CaptureMode::Metadata, CaptureMode::Full] {
        let (pipeline, records) = pipeline(mode);
        pipeline
            .observe(
                &context(),
                Observation::AgentStarted {
                    agent: "agent".into(),
                    instructions: "private configured instructions".into(),
                },
            )
            .await
            .unwrap();
        let records = records.0.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, "agent_start");
        assert_eq!(records[0].data, json!({"agent":"agent"}));
        assert!(
            !String::from_utf8(records[0].to_json_line().unwrap())
                .unwrap()
                .contains("private configured")
        );
    }
}

#[tokio::test]
async fn progress_uses_cumulative_usage_and_bounded_recent_events() {
    let (pipeline, records) = pipeline(CaptureMode::Metadata);
    let context = context();
    pipeline
        .observe(
            &context,
            Observation::AgentStarted {
                agent: "agent".into(),
                instructions: String::new(),
            },
        )
        .await
        .unwrap();
    pipeline
        .observe(
            &context,
            Observation::ToolStarted {
                agent: "agent".into(),
                call: call("t"),
            },
        )
        .await
        .unwrap();
    pipeline
        .observe(
            &context,
            Observation::RawToolOutput {
                call: call("t"),
                output: output("result"),
            },
        )
        .await
        .unwrap();
    let usage = Usage {
        requests: 3,
        input_tokens: 10,
        output_tokens: 2,
        ..Usage::default()
    };
    for _ in 0..2 {
        pipeline
            .observe(
                &context,
                Observation::Usage {
                    usage: usage.clone(),
                    cost: 0.1,
                },
            )
            .await
            .unwrap();
    }
    for _ in 0..25 {
        pipeline
            .observe(
                &context,
                Observation::Retry {
                    model: "m".into(),
                    delay: Duration::from_millis(1),
                },
            )
            .await
            .unwrap();
    }
    let snapshot = pipeline.snapshot().await;
    assert_eq!(snapshot.sequence, 30);
    assert_eq!(snapshot.usage, usage);
    {
        let records = records.0.lock().unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| record.kind == "usage")
                .count(),
            2
        );
        for record in records.iter().filter(|record| record.kind == "usage") {
            assert_eq!(record.data["usage"]["requests"], 3);
        }
    }
    assert_eq!(snapshot.cost, 0.1);
    assert_eq!(
        (
            snapshot.agent_turns,
            snapshot.tool_calls,
            snapshot.tool_results,
            snapshot.retries
        ),
        (1, 1, 1, 25)
    );
    assert_eq!(snapshot.recent.len(), 20);
    assert_eq!(pipeline.health().await.events_written, 30);
    assert!(pipeline.durable_observer());
}

#[tokio::test]
async fn raw_tool_capture_is_not_limited_by_visible_output_cap() {
    let raw = "large private output 🔐 ".repeat(2000);
    assert!(raw.len() > 16 * 1024);
    for mode in [CaptureMode::Full, CaptureMode::Metadata] {
        let (pipeline, records) = pipeline(mode);
        pipeline
            .observe(
                &context(),
                Observation::RawToolOutput {
                    call: call("t"),
                    output: output(&raw),
                },
            )
            .await
            .unwrap();
        let records = records.0.lock().unwrap();
        let data = &records[0].data;
        if mode == CaptureMode::Full {
            assert_eq!(data["output"]["content"][0]["text"], raw);
        } else {
            assert!(data["output"]["sha256"].is_string());
            assert!(!data.to_string().contains("private output"));
        }
    }
}

#[tokio::test]
async fn concurrent_producers_and_sinks_share_one_sequence() {
    let first = Arc::new(Records::default());
    let second = Arc::new(Records::default());
    let pipeline = Arc::new(
        Observability::new(
            "run-1",
            CapturePolicy::default(),
            vec![first.clone(), second.clone()],
        )
        .unwrap(),
    );
    let mut tasks = Vec::new();
    for i in 0..40 {
        let pipeline = pipeline.clone();
        tasks.push(tokio::spawn(async move {
            pipeline
                .publish(&context(), "log", json!({"text":i}))
                .await
                .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let first = first.0.lock().unwrap();
    assert_eq!(*first, *second.0.lock().unwrap());
    assert_eq!(
        first
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        (1..=40).collect::<Vec<_>>()
    );
}

struct Gate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
}
impl EventSink for Gate {
    fn emit<'a>(&'a self, _: &'a EventRecord) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.entered.notify_one();
            self.release.acquire().await.unwrap().forget();
            Ok(())
        })
    }
}
#[tokio::test]
async fn awaited_sink_applies_backpressure_before_next_sink() {
    let gate = Arc::new(Gate {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Semaphore::new(0),
    });
    let records = Arc::new(Records::default());
    let pipeline = Arc::new(
        Observability::new(
            "run-1",
            CapturePolicy::default(),
            vec![gate.clone(), records.clone()],
        )
        .unwrap(),
    );
    let task = tokio::spawn(async move {
        pipeline
            .publish(&context(), "log", json!({"text":"a"}))
            .await
            .unwrap();
    });
    gate.entered.notified().await;
    assert!(!task.is_finished());
    assert!(records.0.lock().unwrap().is_empty());
    gate.release.add_permits(1);
    task.await.unwrap();
    assert_eq!(records.0.lock().unwrap().len(), 1);
}

struct FailingSink;
impl EventSink for FailingSink {
    fn emit<'a>(&'a self, _: &'a EventRecord) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Err(Error::new(ErrorCategory::Host, "secret diagnostic")) })
    }
}
#[tokio::test]
async fn failures_are_visible_fail_closed_and_do_not_echo_sink_content() {
    let records = Arc::new(Records::default());
    let pipeline = Observability::new(
        "run-1",
        CapturePolicy::default(),
        vec![Arc::new(FailingSink), records.clone()],
    )
    .unwrap();
    let error = pipeline
        .publish(&context(), "log", json!({}))
        .await
        .unwrap_err();
    assert_eq!(error.info.category, ErrorCategory::Host);
    assert!(!error.to_string().contains("secret diagnostic"));
    let health = pipeline.health().await;
    assert_eq!(
        (
            health.events_attempted,
            health.events_written,
            health.write_errors
        ),
        (1, 0, 1)
    );
    assert!(!health.last_error.unwrap().contains("secret diagnostic"));
    assert!(records.0.lock().unwrap().is_empty());
    let mut wrong = context();
    wrong.run_id = "wrong".into();
    assert!(pipeline.publish(&wrong, "log", json!({})).await.is_err());
    assert_eq!(pipeline.health().await.events_attempted, 1);
}

#[derive(Default)]
struct TestHost {
    events: Mutex<Vec<RunEvent>>,
    approvals: AtomicUsize,
}
impl Host for TestHost {
    fn emit<'a>(&'a self, _: &'a Context, event: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.events.lock().unwrap().push(event);
            Ok(())
        })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async move {
            self.approvals.fetch_add(1, Ordering::SeqCst);
            Ok(ApprovalDecision::Defer)
        })
    }
}
#[tokio::test]
async fn host_delegates_original_output_and_approval_without_granting_authority() {
    let (pipeline, records) = pipeline(CaptureMode::Metadata);
    let original = Arc::new(TestHost::default());
    let host = ObservedHost {
        host: original.clone(),
        observations: pipeline,
    };
    let event = RunEvent::ToolFinished {
        call_id: "t".into(),
        output: output("private original"),
    };
    host.emit(&context(), event.clone()).await.unwrap();
    assert_eq!(original.events.lock().unwrap()[0], event);
    assert!(
        !records.0.lock().unwrap()[0]
            .data
            .to_string()
            .contains("private original")
    );
    assert_eq!(
        host.approve(
            &context(),
            ApprovalRequest {
                call: call("t"),
                reason: "ask".into()
            }
        )
        .await
        .unwrap(),
        ApprovalDecision::Defer
    );
    assert_eq!(original.approvals.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn real_runner_hooks_and_host_events_produce_ordered_progress() {
    use adk::runtime::{AgentConfig, ModelBinding, Runner, RunnerConfig};
    struct ModelImpl;
    impl Model for ModelImpl {
        fn provider(&self) -> &str {
            "test"
        }
        fn complete<'a>(
            &'a self,
            _: &'a Context,
            _: ModelRequest,
        ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
            Box::pin(async {
                Ok(ModelResponse {
                    snapshot_raw: None,
                    raw: None,
                    items: vec![RunItem::Message {
                        message: Message {
                            role: Role::Assistant,
                            content: vec![Content::Text {
                                text: "private answer".into(),
                            }],
                        },
                    }],
                    usage: Usage {
                        input_tokens: 5,
                        output_tokens: 2,
                        ..Usage::default()
                    },
                    end_turn: Some(true),
                    response_id: None,
                    metadata: Default::default(),
                })
            })
        }
    }
    let (pipeline, records) = pipeline(CaptureMode::Metadata);
    let runner = Runner::new(
        AgentConfig::new("a", ModelBinding::complete("m", Arc::new(ModelImpl))),
        RunnerConfig {
            hooks: Some(pipeline.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let host = ObservedHost {
        host: Arc::new(TestHost::default()),
        observations: pipeline.clone(),
    };
    let result = runner
        .run(
            context(),
            RunRequest {
                input_provenance: Vec::new(),
                input: vec![],
                policy: RunPolicy::default(),
            },
            Arc::new(host),
        )
        .await
        .unwrap();
    assert_eq!(result.result.status, RunStatus::Completed);
    let snapshot = pipeline.snapshot().await;
    assert_eq!(
        (
            snapshot.agent_turns,
            snapshot.model_attempts,
            snapshot.usage.input_tokens
        ),
        (1, 1, 5)
    );
    let records = records.0.lock().unwrap();
    assert_eq!(records.first().unwrap().kind, "run_start");
    assert_eq!(records.last().unwrap().kind, "done");
    for (index, record) in records.iter().enumerate() {
        assert_eq!(record.sequence, index as u64 + 1);
        assert!(!record.data.to_string().contains("private answer"));
    }
}

#[tokio::test]
async fn terminal_delivery_and_shutdown_are_explicit() {
    let (pipeline, _) = pipeline(CaptureMode::Metadata);
    pipeline
        .publish(&context(), "done", json!({}))
        .await
        .unwrap();
    assert!(
        pipeline
            .publish(&context(), "log", json!({}))
            .await
            .is_err()
    );
    pipeline.shutdown().await.unwrap();
    pipeline.shutdown().await.unwrap();
    let (pipeline, _) = self::pipeline(CaptureMode::Metadata);
    pipeline.shutdown().await.unwrap();
    assert!(
        pipeline
            .publish(&context(), "log", json!({}))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn real_two_turn_tool_run_preserves_pause_continuation_and_success_spans() {
    use adk::runtime::{AgentConfig, ModelBinding, Runner, RunnerConfig};
    struct TwoTurns(AtomicUsize);
    impl Model for TwoTurns {
        fn provider(&self) -> &str {
            "test"
        }
        fn complete<'a>(
            &'a self,
            _: &'a Context,
            _: ModelRequest,
        ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
            Box::pin(async move {
                let first = self.0.fetch_add(1, Ordering::SeqCst) == 0;
                Ok(ModelResponse {
                    snapshot_raw: None,
                    raw: None,
                    items: if first {
                        vec![RunItem::ToolCall { call: call("t") }]
                    } else {
                        vec![RunItem::Message {
                            message: Message {
                                role: Role::Assistant,
                                content: vec![Content::Text {
                                    text: "done".into(),
                                }],
                            },
                        }]
                    },
                    usage: Usage {
                        input_tokens: 2,
                        output_tokens: 1,
                        ..Usage::default()
                    },
                    end_turn: Some(!first),
                    response_id: None,
                    metadata: Default::default(),
                })
            })
        }
    }
    struct PauseTool {
        definition: ToolDefinition,
        pause: bool,
    }
    impl Tool for PauseTool {
        fn definition(&self) -> &ToolDefinition {
            &self.definition
        }
        fn execute<'a>(
            &'a self,
            _: &'a ToolContext,
            _: ToolCall,
        ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
            Box::pin(async move {
                let mut output = output("tool result");
                output.should_pause = self.pause;
                Ok(output)
            })
        }
    }
    for pause in [false, true] {
        let records = Arc::new(Records::default());
        let sinks: Vec<Arc<dyn EventSink>> = vec![records.clone()];
        #[cfg(feature = "otel")]
        let (sinks, exporter, provider) = {
            use opentelemetry::trace::TracerProvider;
            use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
            let exporter = InMemorySpanExporter::default();
            let provider = SdkTracerProvider::builder()
                .with_simple_exporter(exporter.clone())
                .build();
            let mut sinks = sinks;
            sinks.push(Arc::new(adk::observability::otel::OtelBridge::new(
                provider.tracer("pause-test"),
                opentelemetry::Context::new(),
            )));
            (sinks, exporter, provider)
        };
        let pipeline =
            Arc::new(Observability::new("run-1", CapturePolicy::default(), sinks).unwrap());
        let model = Arc::new(TwoTurns(AtomicUsize::new(0)));
        let mut agent = AgentConfig::new("a", ModelBinding::complete("m", model.clone()));
        agent.tools.push(Arc::new(PauseTool {
            definition: ToolDefinition {
                name: "test".into(),
                description: String::new(),
                input_schema: true.into(),
                read_only: true,
                requires_approval: false,
            },
            pause,
        }));
        let runner = Runner::new(
            agent,
            RunnerConfig {
                hooks: Some(pipeline.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let host = Arc::new(ObservedHost {
            host: Arc::new(TestHost::default()),
            observations: pipeline.clone(),
        });
        let mut result = runner
            .run(
                context(),
                RunRequest {
                    input_provenance: Vec::new(),
                    input: vec![],
                    policy: RunPolicy::default(),
                },
                host,
            )
            .await
            .unwrap();
        if pause {
            assert_eq!(result.result.status, RunStatus::Paused);
            #[cfg(feature = "otel")]
            assert!(
                exporter
                    .get_finished_spans()
                    .unwrap()
                    .iter()
                    .all(|span| span.name != "adk.run" && span.name != "adk.agent")
            );
            result = result.continuation.unwrap().resume(None).await.unwrap();
        }
        assert_eq!(result.result.status, RunStatus::Completed);
        assert_eq!(model.0.load(Ordering::SeqCst), 2);
        assert_eq!(pipeline.snapshot().await.agent_turns, 2);
        pipeline.shutdown().await.unwrap();
        let records = records.0.lock().unwrap();
        assert_eq!(
            records.iter().filter(|r| r.kind == "done").count(),
            if pause { 2 } else { 1 }
        );
        for (index, record) in records.iter().enumerate() {
            assert_eq!(record.sequence, index as u64 + 1);
        }
        #[cfg(feature = "otel")]
        {
            provider.force_flush().unwrap();
            let spans = exporter.get_finished_spans().unwrap();
            assert_eq!(spans.iter().filter(|s| s.name == "adk.run").count(), 1);
            assert_eq!(spans.iter().filter(|s| s.name == "adk.agent").count(), 1);
            assert_eq!(spans.iter().filter(|s| s.name == "gen_ai.chat").count(), 2);
            assert!(
                spans
                    .iter()
                    .all(|s| !matches!(s.status, opentelemetry::trace::Status::Error { .. })),
                "{spans:?}"
            );
            provider.shutdown().unwrap();
        }
    }
}

#[cfg(unix)]
mod filesystem {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };

    fn private_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        root
    }

    #[test]
    fn private_trace_layout_round_trips_and_never_reopens_existing_run() {
        let root = private_root();
        let store =
            FilesystemTraceStore::create(root.path(), "run-1", TraceLimits::default()).unwrap();
        store.append(&record(1)).unwrap();
        let path = root.path().join("traces/run-1/events.jsonl");
        assert_eq!(
            fs::metadata(root.path().join("traces"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(root.path().join("traces/run-1"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            EventRecord::from_json_line(&fs::read(&path).unwrap()).unwrap(),
            record(1)
        );
        assert!(
            FilesystemTraceStore::create(root.path(), "run-1", TraceLimits::default()).is_err()
        );
        assert!(store.append(&record(1)).is_err());
        let mut wrong = record(2);
        wrong.run_id = "wrong".into();
        assert!(store.append(&wrong).is_err());
        assert_eq!(fs::read(&path).unwrap(), record(1).to_json_line().unwrap());
    }

    #[test]
    fn traversal_symlinks_and_public_roots_are_rejected() {
        let root = private_root();
        for name in ["", ".", "..", "../escape", "a/b", "a\\b", "☃"] {
            assert!(
                FilesystemTraceStore::create(root.path(), name, TraceLimits::default()).is_err()
            );
        }
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("link")).unwrap();
        assert!(
            FilesystemTraceStore::create(
                root.path().join("link/private"),
                "run-1",
                TraceLimits::default()
            )
            .is_err()
        );
        assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
        symlink(outside.path(), root.path().join("traces")).unwrap();
        assert!(
            FilesystemTraceStore::create(root.path(), "run-1", TraceLimits::default()).is_err()
        );
        fs::remove_file(root.path().join("traces")).unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            FilesystemTraceStore::create(root.path(), "run-1", TraceLimits::default()).is_err()
        );
    }

    #[test]
    fn quotas_rotate_whole_lines_and_reject_without_modifying_existing_chunks() {
        let root = private_root();
        let bytes = record(1).to_json_line().unwrap().len();
        let store = FilesystemTraceStore::create(
            root.path(),
            "run-1",
            TraceLimits {
                event_bytes: bytes,
                chunk_bytes: bytes as u64,
                rotations: 1,
            },
        )
        .unwrap();
        store.append(&record(1)).unwrap();
        store.append(&record(2)).unwrap();
        assert!(
            store
                .append(&record(3))
                .unwrap_err()
                .to_string()
                .contains("quota")
        );
        let run = root.path().join("traces/run-1");
        assert_eq!(
            fs::read(run.join("events.jsonl")).unwrap(),
            record(1).to_json_line().unwrap()
        );
        assert_eq!(
            fs::read(run.join("events.jsonl.001")).unwrap(),
            record(2).to_json_line().unwrap()
        );
        let mut oversized = record(3);
        oversized.data = json!({"text":"x".repeat(bytes)});
        assert!(
            store
                .append(&oversized)
                .unwrap_err()
                .to_string()
                .contains("quota")
        );
        assert_eq!(fs::read_dir(run).unwrap().count(), 2);
    }

    #[test]
    fn preexisting_rotation_symlink_and_hardlink_are_never_written_through() {
        for hardlink in [false, true] {
            let root = private_root();
            let bytes = record(1).to_json_line().unwrap().len();
            let store = FilesystemTraceStore::create(
                root.path(),
                "run-1",
                TraceLimits {
                    event_bytes: bytes,
                    chunk_bytes: bytes as u64,
                    rotations: 1,
                },
            )
            .unwrap();
            store.append(&record(1)).unwrap();
            let victim = root.path().join("victim");
            fs::write(&victim, "untouched").unwrap();
            let rotation = root.path().join("traces/run-1/events.jsonl.001");
            if hardlink {
                fs::hard_link(&victim, &rotation).unwrap();
            } else {
                symlink(&victim, &rotation).unwrap();
            }
            assert!(store.append(&record(2)).is_err());
            assert_eq!(fs::read_to_string(victim).unwrap(), "untouched");
        }
    }

    #[test]
    fn pinned_descriptors_do_not_follow_replaced_root_paths() {
        let root = private_root();
        let private = root.path().join("private");
        let store =
            FilesystemTraceStore::create(&private, "run-1", TraceLimits::default()).unwrap();
        let moved = root.path().join("moved");
        fs::rename(&private, &moved).unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), &private).unwrap();
        store.append(&record(1)).unwrap();
        assert_eq!(
            fs::read(moved.join("traces/run-1/events.jsonl")).unwrap(),
            record(1).to_json_line().unwrap()
        );
        assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
    }
}

#[cfg(feature = "otel")]
mod telemetry {
    use super::*;
    use adk::observability::otel::OtelBridge;
    use opentelemetry::{
        Context as OtelContext,
        trace::{Status, TraceContextExt, Tracer, TracerProvider},
    };
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};

    fn attribute(span: &SpanData, key: &str) -> Option<String> {
        span.attributes
            .iter()
            .find(|kv| kv.key.as_str() == key)
            .map(|kv| kv.value.to_string())
    }

    #[tokio::test]
    async fn actual_sdk_exports_parented_spans_final_usage_errors_and_redacted_content() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let tracer = provider.tracer("adk-tests");
        let parent = OtelContext::new().with_span(tracer.start("host-parent"));
        let parent_id = parent.span().span_context().span_id();
        let pipeline = Observability::new(
            "run-1",
            CapturePolicy::default(),
            vec![Arc::new(OtelBridge::new(tracer, parent.clone()))],
        )
        .unwrap();
        let context = context();
        pipeline
            .observe(
                &context,
                Observation::AgentStarted {
                    agent: "a".into(),
                    instructions: String::new(),
                },
            )
            .await
            .unwrap();
        pipeline
            .observe(
                &context,
                Observation::ModelAttempt {
                    agent: "a".into(),
                    model: "m".into(),
                    attempt: 1,
                },
            )
            .await
            .unwrap();
        pipeline
            .observe(
                &context,
                Observation::ModelAccepted {
                    agent: "a".into(),
                    response: ModelResponse {
                        snapshot_raw: None,
                        raw: None,
                        items: vec![],
                        usage: Usage {
                            input_tokens: 11,
                            output_tokens: 3,
                            ..Usage::default()
                        },
                        end_turn: Some(false),
                        response_id: None,
                        metadata: Default::default(),
                    },
                },
            )
            .await
            .unwrap();
        pipeline
            .observe(
                &context,
                Observation::ToolStarted {
                    agent: "a".into(),
                    call: call("t"),
                },
            )
            .await
            .unwrap();
        let mut failed = output("private failure output");
        failed.is_error = true;
        pipeline
            .observe(
                &context,
                Observation::RawToolOutput {
                    call: call("t"),
                    output: failed,
                },
            )
            .await
            .unwrap();
        pipeline
            .record_run_event(
                &context,
                &RunEvent::Failed {
                    error: ErrorInfo {
                        category: ErrorCategory::Provider,
                        message: "private final error".into(),
                    },
                },
            )
            .await
            .unwrap();
        pipeline.shutdown().await.unwrap();
        provider.force_flush().unwrap();
        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 4);
        let root = spans.iter().find(|s| s.name == "adk.run").unwrap();
        let agent = spans.iter().find(|s| s.name == "adk.agent").unwrap();
        let generation = spans.iter().find(|s| s.name == "gen_ai.chat").unwrap();
        let tool = spans.iter().find(|s| s.name == "adk.tool").unwrap();
        assert_eq!(root.parent_span_id, parent_id);
        assert_eq!(agent.parent_span_id, root.span_context.span_id());
        assert_eq!(generation.parent_span_id, agent.span_context.span_id());
        assert_eq!(tool.parent_span_id, agent.span_context.span_id());
        assert!(
            spans
                .iter()
                .all(|s| s.span_context.trace_id() == root.span_context.trace_id())
        );
        assert_eq!(
            attribute(generation, "gen_ai.usage.input_tokens").as_deref(),
            Some("11")
        );
        assert_eq!(attribute(tool, "tool.error").as_deref(), Some("true"));
        assert_eq!(attribute(root, "error.type").as_deref(), Some("provider"));
        assert!(matches!(root.status, Status::Error { .. }));
        assert!(matches!(tool.status, Status::Error { .. }));
        assert!(!format!("{spans:?}").contains("private failure output"));
        assert!(!format!("{spans:?}").contains("private final error"));
        parent.span().end();
        provider.shutdown().unwrap();
    }

    #[tokio::test]
    async fn child_can_parent_to_ended_tool_and_shutdown_ends_unfinished_spans() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let bridge = Arc::new(OtelBridge::new(
            provider.tracer("adk-tests"),
            OtelContext::new(),
        ));
        let pipeline = Observability::new("run-1", CapturePolicy::default(), vec![bridge]).unwrap();
        pipeline
            .observe(
                &context(),
                Observation::ToolStarted {
                    agent: "a".into(),
                    call: call("parent"),
                },
            )
            .await
            .unwrap();
        pipeline
            .observe(
                &context(),
                Observation::RawToolOutput {
                    call: call("parent"),
                    output: output("complete"),
                },
            )
            .await
            .unwrap();
        pipeline.publish(&context(), "tool_start", json!({"call_id":"child","parent_call_id":"parent","tool_name":"nested","input":{}})).await.unwrap();
        pipeline.shutdown().await.unwrap();
        provider.force_flush().unwrap();
        let spans = exporter.get_finished_spans().unwrap();
        let parent = spans
            .iter()
            .find(|s| attribute(s, "tool.call.id").as_deref() == Some("parent"))
            .unwrap();
        let child = spans
            .iter()
            .find(|s| attribute(s, "tool.call.id").as_deref() == Some("child"))
            .unwrap();
        assert_eq!(child.parent_span_id, parent.span_context.span_id());
        assert!(matches!(child.status, Status::Error { .. }));
        assert_eq!(spans.len(), 3);
        assert!(parent.end_time <= child.end_time);
        provider.shutdown().unwrap();
    }

    #[tokio::test]
    async fn bridge_isolates_runs_sharing_tool_ids() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let bridge = Arc::new(OtelBridge::new(
            provider.tracer("adk-tests"),
            OtelContext::new(),
        ));
        for run in ["run-1", "run-2"] {
            let pipeline =
                Observability::new(run, CapturePolicy::default(), vec![bridge.clone()]).unwrap();
            let mut context = context();
            context.run_id = run.into();
            pipeline
                .observe(
                    &context,
                    Observation::ToolStarted {
                        agent: "a".into(),
                        call: call("same"),
                    },
                )
                .await
                .unwrap();
        }
        bridge.shutdown().await.unwrap();
        provider.force_flush().unwrap();
        let spans = exporter.get_finished_spans().unwrap();
        let tools = spans
            .iter()
            .filter(|s| s.name == "adk.tool")
            .collect::<Vec<_>>();
        assert_eq!(tools.len(), 2);
        assert_ne!(
            tools[0].span_context.trace_id(),
            tools[1].span_context.trace_id()
        );
        assert_ne!(tools[0].parent_span_id, tools[1].parent_span_id);
        provider.shutdown().unwrap();
    }

    #[tokio::test]
    async fn closing_one_pipeline_does_not_end_another_runs_shared_bridge_spans() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let bridge = Arc::new(OtelBridge::new(
            provider.tracer("shared-test"),
            OtelContext::new(),
        ));
        let a =
            Observability::new("run-1", CapturePolicy::default(), vec![bridge.clone()]).unwrap();
        let b =
            Observability::new("run-2", CapturePolicy::default(), vec![bridge.clone()]).unwrap();
        let mut b_context = context();
        b_context.run_id = "run-2".into();
        for (pipeline, context) in [(&a, context()), (&b, b_context.clone())] {
            pipeline
                .observe(
                    &context,
                    Observation::ToolStarted {
                        agent: "a".into(),
                        call: call("t"),
                    },
                )
                .await
                .unwrap();
        }
        a.shutdown().await.unwrap();
        let first = exporter.get_finished_spans().unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(
            first
                .iter()
                .find_map(|s| attribute(s, "adk.run.id"))
                .as_deref(),
            Some("run-1")
        );
        b.observe(
            &b_context,
            Observation::RawToolOutput {
                call: call("t"),
                output: output("b finished"),
            },
        )
        .await
        .unwrap();
        b.publish(&b_context, "done", json!({"status":"completed"}))
            .await
            .unwrap();
        b.shutdown().await.unwrap();
        bridge.shutdown().await.unwrap();
        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 4);
        let b_root = spans
            .iter()
            .find(|s| attribute(s, "adk.run.id").as_deref() == Some("run-2"))
            .unwrap();
        let b_spans = spans
            .iter()
            .filter(|s| s.span_context.trace_id() == b_root.span_context.trace_id())
            .collect::<Vec<_>>();
        assert_eq!(b_spans.len(), 2);
        assert!(
            b_spans
                .iter()
                .all(|s| !matches!(s.status, Status::Error { .. }))
        );
        assert_eq!(
            attribute(
                b_spans.iter().find(|s| s.name == "adk.tool").unwrap(),
                "tool.error"
            )
            .as_deref(),
            Some("false")
        );
        provider.shutdown().unwrap();
    }
}
