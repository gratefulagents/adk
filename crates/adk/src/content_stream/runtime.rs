use super::ContentStream;
use crate::tracewriter::{SpanData, generation_span};
use adk_codec::dto::ContentEvent;
use adk_core::{Context, RunItem};
use adk_runtime::tracing::{GenerationObserver, GenerationRecord, GenerationStatus};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Host-attached content observer; live reasoning requires `RunnerConfig::blocking_reasoning`.
/// Attempt numbers retain native runtime turn numbering. Content is not trace-redacted.
///
/// ```
/// use adk::{content_stream::{ContentGenerationObserver, ContentStream}, runtime::RunnerConfig};
/// use std::sync::Arc;
/// let stream = Arc::new(ContentStream::new(std::io::sink()));
/// let config = RunnerConfig {
///     generation_observer: Some(Arc::new(ContentGenerationObserver::new(stream.clone()))),
///     blocking_reasoning: true,
///     ..Default::default()
/// };
/// // Pass config to Runner::new or AgentBuilder::runner_config.
/// // After the owned run completes, explicitly flush the host's writer.
/// stream.flush()?;
/// # let _ = config;
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct ContentGenerationObserver {
    stream: Arc<ContentStream>,
    reasoning: Mutex<HashMap<String, ThinkingBuffer>>,
}

impl ContentGenerationObserver {
    pub fn new(stream: Arc<ContentStream>) -> Self {
        Self {
            stream,
            reasoning: Mutex::new(HashMap::new()),
        }
    }

    fn emit_chunk(&self, record: &GenerationRecord, chunk: Option<String>) {
        if let Some(chunk) = chunk {
            self.stream.emit(ContentEvent {
                kind: "assistant_thinking_delta".into(),
                message: chunk,
                agent_name: record.agent.clone(),
                tool_use_id: record.id.clone(),
                task_id: record.task_id.clone().unwrap_or_default(),
                ..Default::default()
            });
        }
    }
}

impl GenerationObserver for ContentGenerationObserver {
    fn start(&self, context: &Context, record: &GenerationRecord) {
        self.reasoning
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(record.id.clone(), ThinkingBuffer::default());
        self.stream.emit_llm_attempt(attempt(context, record));
    }

    fn reasoning_delta(&self, _: &Context, record: &GenerationRecord, delta: &str) {
        let chunk = self
            .reasoning
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(&record.id)
            .and_then(|buffer| buffer.chunk(delta, Instant::now()));
        self.emit_chunk(record, chunk);
    }

    fn returned(&self, _: &Context, record: &GenerationRecord) {
        let chunk = self
            .reasoning
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(&record.id)
            .and_then(|buffer| buffer.flush(Instant::now()));
        self.emit_chunk(record, chunk);
    }

    fn end(&self, context: &Context, record: &GenerationRecord) {
        let chunk = self
            .reasoning
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&record.id)
            .and_then(|mut buffer| buffer.flush(Instant::now()));
        self.emit_chunk(record, chunk);
        self.stream.emit_llm_attempt(attempt(context, record));
    }

    fn accepted(&self, _: &Context, record: &GenerationRecord) {
        if let Some(response) = &record.response {
            for item in &response.items {
                if let RunItem::Reasoning { reasoning } = item {
                    if !reasoning.text.is_empty() {
                        self.stream.emit(ContentEvent {
                            kind: "assistant_thinking".into(),
                            message: reasoning.text.clone(),
                            agent_name: record.agent.clone(),
                            tool_use_id: record.id.clone(),
                            task_id: record.task_id.clone().unwrap_or_default(),
                            ..Default::default()
                        });
                    }
                }
            }
        }
    }
}

fn attempt(context: &Context, record: &GenerationRecord) -> ContentEvent {
    let Some(SpanData::Generation(generation)) = generation_span(context, record).data else {
        unreachable!("generation projection")
    };
    let mut event = ContentEvent {
        tool_use_id: record.id.clone(),
        agent_name: record.agent.clone(),
        attempt_number: generation.attempt_number as i32,
        turn: generation.generation_turn as i32,
        attempt_status: if record.status == GenerationStatus::Started {
            "started".into()
        } else {
            generation.status
        },
        scope: generation.scope,
        task_id: generation.task_id,
        requested_model: generation.requested_model,
        resolved_model: generation.resolved_model,
        canonical_model: generation.model_canonical,
        provider: generation.model_provider,
        retry_planned: matches!(
            record.status,
            GenerationStatus::Retrying | GenerationStatus::Fallback
        ),
        retry_after_ms: generation.retry_after_ms,
        fallback_planned: generation.fallback_scheduled,
        fallback_from_model: generation.fallback_from_model,
        fallback_to_model: generation.fallback_to_model,
        fallback_reason: generation.fallback_reason,
        ..Default::default()
    };
    if record.status == GenerationStatus::Completed {
        event.usage_available = generation.usage_available;
        event.has_prompt_tokens = generation.usage_available;
        event.has_completion_tokens = generation.usage_available;
        event.has_total_tokens = generation.usage_available;
        event.input_tokens = generation.prompt_tokens;
        event.output_tokens = generation.completion_tokens;
        event.total_tokens = generation.total_tokens;
        event.cache_read_input_tokens = generation.cache_read_tokens;
        event.cache_creation_input_tokens = generation.cache_creation_tokens;
        event.input_tokens_include_cache = generation.input_tokens_include_cache;
        event.input_tokens_include_cache_known = generation.input_tokens_include_cache_known;
        event.cost_usd = generation.cost_usd;
        event.cost_known = generation.cost_known;
        event.attempt_latency_ms = generation.latency_ms;
    } else if record.status != GenerationStatus::Started {
        event.attempt_latency_ms = generation.latency_ms;
        event.failure_kind = generation.failure_kind;
        event.reason.clone_from(&event.failure_kind);
        event.message.clone_from(&event.failure_kind);
        event.is_error = !generation.error.is_empty();
        event.output = super::truncate_tool_content(generation.error);
    }
    event
}

#[derive(Default)]
struct ThinkingBuffer {
    text: String,
    last_flush: Option<Instant>,
}

impl ThinkingBuffer {
    fn chunk(&mut self, text: &str, now: Instant) -> Option<String> {
        if text.is_empty() {
            return None;
        }
        self.text.push_str(text);
        if self.text.len() >= 4096
            || self.last_flush.is_none_or(|last| {
                now.saturating_duration_since(last) >= Duration::from_millis(500)
            })
        {
            self.flush(now)
        } else {
            None
        }
    }

    fn flush(&mut self, now: Instant) -> Option<String> {
        self.last_flush = Some(now);
        if self.text.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.text))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalescing_matches_pinned_sdk_fake_clock_cases() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../fixtures/content-stream/observations.json"
        ))
        .unwrap();
        let base = Instant::now();
        for case in fixture["thinkingCases"].as_array().unwrap() {
            let mut buffer = ThinkingBuffer::default();
            for observation in case["observations"].as_array().unwrap() {
                let action = &observation["action"];
                let now = base + Duration::from_nanos(action["afterNanos"].as_u64().unwrap());
                let chunk = if action["flush"].as_bool().unwrap() {
                    buffer.flush(now)
                } else {
                    buffer.chunk(action["text"].as_str().unwrap(), now)
                };
                assert_eq!(
                    serde_json::json!(chunk.into_iter().collect::<Vec<_>>()),
                    observation["chunks"],
                    "{}: {action}",
                    case["name"]
                );
            }
        }
    }

    fn record() -> GenerationRecord {
        GenerationRecord {
            id: "attempt".into(),
            agent: "worker".into(),
            provider: " MOCK ".into(),
            resolved_model: "resolved".into(),
            input_tokens_include_cache: Some(true),
            task_id: Some("task".into()),
            cost_usd: Some(1.25),
            turn: 7,
            request: adk_core::ModelRequest {
                model: " MOCK/model ".into(),
                instructions: String::new(),
                input: vec![],
                input_provenance: vec![],
                tools: vec![],
                output_schema: None,
                output_schema_name: String::new(),
                output_schema_strict: false,
                settings: Default::default(),
            },
            declared_tool_timeouts: vec![],
            request_snapshot: Err(adk_codec::approval::BridgeError("test")),
            response: None,
            error: Some(adk_core::ErrorInfo {
                category: adk_core::ErrorCategory::Internal,
                message: "😀".repeat(10_000),
            }),
            retry_reason: Some("429".into()),
            status: GenerationStatus::Started,
            retry_after: None,
            fallback_model: None,
            started_at: std::time::SystemTime::now(),
            ended_at: None,
            latency: Duration::from_millis(17),
        }
    }

    fn context() -> Context {
        Context {
            run_id: "run".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        }
    }

    #[test]
    fn projection_uses_status_for_retry_and_bounds_failure_content() {
        let mut record = record();
        let context = context();
        let started = attempt(&context, &record);
        assert_eq!(started.attempt_status, "started");
        assert_eq!(started.canonical_model, "mock/model");
        assert_eq!(started.provider, "mock");
        assert_eq!(started.resolved_model, "resolved");
        assert_eq!(started.attempt_number, 7);
        assert_eq!(started.turn, 7);
        assert_eq!(started.scope, "subagent");
        assert_eq!(started.task_id, "task");
        assert!(!started.input_tokens_include_cache_known);
        assert!(!started.cost_known);
        assert!(!started.is_error);
        assert_eq!(started.attempt_latency_ms, 0);
        for status in [
            GenerationStatus::Retrying,
            GenerationStatus::Fallback,
            GenerationStatus::Failed,
            GenerationStatus::Interrupted,
        ] {
            record.status = status;
            record.fallback_model = (status == GenerationStatus::Fallback).then(|| "next".into());
            let event = attempt(&context, &record);
            assert_eq!(
                event.retry_planned,
                matches!(
                    status,
                    GenerationStatus::Retrying | GenerationStatus::Fallback
                )
            );
            assert_eq!(event.retry_after_ms, 0);
            assert_eq!(event.attempt_latency_ms, 17);
            assert!(event.is_error);
            assert_eq!(event.reason, event.failure_kind);
            assert_eq!(event.message, event.failure_kind);
            assert!(event.output.len() <= super::super::DEFAULT_MAX_TOOL_RESULT_BYTES);
            assert!(event.output.ends_with("..."));
            if status == GenerationStatus::Fallback {
                assert!(event.fallback_planned);
                assert_eq!(event.fallback_from_model, "MOCK/model");
                assert_eq!(event.fallback_to_model, "next");
                assert_eq!(event.fallback_reason, "rate_limit");
            }
        }
    }

    #[test]
    fn returned_and_end_flush_each_attempt_once_and_remove_buffers() {
        let stream = Arc::new(ContentStream::new(std::io::sink()));
        let subscription = stream.subscribe(20);
        let observer = ContentGenerationObserver::new(stream);
        let context = context();
        let first = record();
        let mut second = first.clone();
        second.id = "second".into();
        for record in [&first, &second] {
            observer.start(&context, record);
            observer.reasoning_delta(&context, record, "first");
            observer.reasoning_delta(&context, record, "residue");
        }
        observer.returned(&context, &first);
        observer.returned(&context, &first);
        observer.end(&context, &first);
        observer.end(&context, &second);
        assert!(observer.reasoning.lock().unwrap().is_empty());
        let chunks: Vec<_> = subscription
            .receiver
            .try_iter()
            .filter(|e| e.kind == "assistant_thinking_delta")
            .map(|e| (e.tool_use_id, e.message))
            .collect();
        assert_eq!(
            chunks,
            [
                ("attempt".into(), "first".into()),
                ("second".into(), "first".into()),
                ("attempt".into(), "residue".into()),
                ("second".into(), "residue".into())
            ]
        );
        let mut buffer = ThinkingBuffer::default();
        let now = Instant::now();
        assert_eq!(
            buffer.chunk("first", now + Duration::from_secs(1)),
            Some("first".into())
        );
        assert_eq!(buffer.chunk("backward", now), None);
        assert_eq!(buffer.flush(now), Some("backward".into()));
    }
}
