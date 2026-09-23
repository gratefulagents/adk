use crate::tracewriter::{Span, SpanData, Trace, generation_span};
use opentelemetry::{
    Context, KeyValue,
    trace::{Status, TraceContextExt, Tracer},
};
use opentelemetry_sdk::trace::SdkTracer;
use regex::Regex;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

#[derive(Default)]
struct State {
    active: HashMap<String, Context>,
    parents: HashMap<String, Context>,
    root: Option<Context>,
    trace_id: String,
    callback: Option<Arc<dyn Fn(String) + Send + Sync>>,
    notified: bool,
}

/// SDK span mapping, separate from the native ordered-event bridge.
/// Ended parent contexts survive until trace end for delayed child spans.
pub struct SpanProcessor {
    tracer: SdkTracer,
    state: Mutex<State>,
}
impl SpanProcessor {
    pub fn new(tracer: SdkTracer) -> Self {
        Self {
            tracer,
            state: Mutex::new(State::default()),
        }
    }
    pub fn trace_id(&self) -> String {
        self.state
            .lock()
            .expect("span processor poisoned")
            .trace_id
            .clone()
    }
    pub fn set_on_trace_id_ready(&self, callback: Option<Arc<dyn Fn(String) + Send + Sync>>) {
        self.state.lock().expect("span processor poisoned").callback = callback;
    }
    pub fn on_trace_start(&self, trace: &Trace) {
        let span = self.tracer.build_with_context(
            self.tracer
                .span_builder(trace.name.clone())
                .with_attributes([KeyValue::new("trace.id", trace.id.clone())]),
            &Context::new(),
        );
        let context = Context::new().with_span(span);
        let trace_id = context.span().span_context().trace_id().to_string();
        let callback = {
            let mut state = self.state.lock().expect("span processor poisoned");
            state.active.insert(trace.id.clone(), context.clone());
            state.parents.insert(trace.id.clone(), context.clone());
            state.root = Some(context);
            state.trace_id = trace_id.clone();
            if !state.notified && state.callback.is_some() {
                state.notified = true;
                state.callback.clone()
            } else {
                None
            }
        };
        if let Some(callback) = callback {
            callback(trace_id);
        }
    }
    pub fn on_trace_end(&self, trace: &Trace) {
        let context = {
            let mut state = self.state.lock().expect("span processor poisoned");
            let context = state.active.remove(&trace.id);
            if let Some(context) = &context {
                let trace_id = context.span().span_context().trace_id();
                // A host may share this processor across overlapping trace scopes.
                // Retire only this root's parents, not another live trace's contexts.
                state
                    .parents
                    .retain(|_, parent| parent.span().span_context().trace_id() != trace_id);
            }
            context
        };
        if let Some(context) = context {
            context.span().end();
        }
    }
    pub fn on_span_start(&self, span: &Span) {
        let mut state = self.state.lock().expect("span processor poisoned");
        let parent = state
            .parents
            .get(&span.parent_id)
            .or(state.root.as_ref())
            .cloned()
            .unwrap_or_default();
        let (name, _) = map_span(span);
        let otel_span = self
            .tracer
            .build_with_context(self.tracer.span_builder(name), &parent);
        let context = parent.with_span(otel_span);
        state.active.insert(span.id.clone(), context.clone());
        state.parents.insert(span.id.clone(), context);
    }
    pub fn on_span_end(&self, span: &Span) {
        let context = self
            .state
            .lock()
            .expect("span processor poisoned")
            .active
            .remove(&span.id);
        if let Some(context) = context {
            let (_, attributes) = map_span(span);
            // Rust SDK retains duplicate attribute keys; emit final values once.
            context.span().set_attributes(attributes);
            context.span().set_attributes([
                KeyValue::new("span.id", span.id.clone()),
                KeyValue::new("span.parent_id", span.parent_id.clone()),
            ]);
            context
                .span()
                .set_attribute(KeyValue::new("duration_ms", span.duration_ms()));
            let error = match &span.data {
                Some(SpanData::Function {
                    is_error: true,
                    output,
                    ..
                }) => Some(output.as_str()),
                Some(SpanData::Subagent(data)) if data.status == "failed" => {
                    Some(data.result_text.as_str())
                }
                Some(SpanData::Generation(data))
                    if !data.success && (!data.error.is_empty() || data.status == "failed") =>
                {
                    Some(data.error.as_str())
                }
                _ => None,
            };
            if let Some(error) = error {
                context.span().set_attribute(KeyValue::new("error", true));
                context
                    .span()
                    .set_status(Status::error(redact(&truncate(error, 256))));
            }
            context.span().end();
        }
    }
}
fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.into();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
}
fn redact(text: &str) -> String {
    static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [r"(?i)(bearer\s+)[A-Za-z0-9._\-]+", r#"(?i)("(?:access_token|refresh_token|id_token|api_key|authorization|password|secret|token)"\s*:\s*")[^"]+(\")"#, r"\bsk-[A-Za-z0-9_-]+", r"\bgh[pousr]_[A-Za-z0-9_]+\b"].iter().map(|pattern| Regex::new(pattern).expect("static OTel redactor")).collect()
    });
    let mut text = text.to_owned();
    for pattern in PATTERNS.iter() {
        text = pattern
            .replace_all(
                &text,
                if pattern.captures_len() == 3 {
                    "${1}[REDACTED]${2}"
                } else {
                    "[REDACTED]"
                },
            )
            .into_owned();
    }
    text
}
fn map_span(span: &Span) -> (String, Vec<KeyValue>) {
    let Some(data) = &span.data else {
        return (span.name.clone(), vec![]);
    };
    let mut attrs = Vec::new();
    let value = serde_json::to_value(data).expect("serializable span data");
    let mut fields = |prefix: &str, names: &[(&str, &str)]| {
        for (target, source) in names {
            let key = format!("{prefix}.{target}");
            match &value[*source] {
                serde_json::Value::String(s) => attrs.push(KeyValue::new(key, s.clone())),
                serde_json::Value::Bool(b) => attrs.push(KeyValue::new(key, *b)),
                serde_json::Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        attrs.push(KeyValue::new(key, i));
                    } else if let Some(f) = n.as_f64() {
                        attrs.push(KeyValue::new(key, f));
                    }
                }
                _ => {}
            }
        }
    };
    let name = match data {
        SpanData::Agent {
            agent_name,
            instructions,
        } => {
            attrs.extend([
                KeyValue::new("agent.name", agent_name.clone()),
                KeyValue::new("agent.instructions", redact(&truncate(instructions, 500))),
            ]);
            format!("agent.{agent_name}")
        }
        SpanData::Function {
            tool_name,
            input,
            output,
            is_error,
        } => {
            attrs.extend([
                KeyValue::new("tool.name", tool_name.clone()),
                KeyValue::new("tool.input", redact(&truncate(input, 1000))),
                KeyValue::new("tool.output", redact(&truncate(output, 1000))),
                KeyValue::new("tool.error", *is_error),
            ]);
            format!("tool.{tool_name}")
        }
        SpanData::Generation(g) => {
            fields(
                "gen",
                &[
                    ("requested_model", "requested_model"),
                    ("resolved_model", "resolved_model"),
                    ("model_provider", "model_provider"),
                    ("model_canonical", "model_canonical"),
                    ("attempt_number", "attempt_number"),
                    ("turn", "generation_turn"),
                    ("scope", "scope"),
                    ("task_id", "task_id"),
                    ("status", "status"),
                    ("usage_available", "usage_available"),
                    ("input_tokens", "prompt_tokens"),
                    ("output_tokens", "completion_tokens"),
                    ("prompt_tokens", "prompt_tokens"),
                    ("completion_tokens", "completion_tokens"),
                    ("cache_read_tokens", "cache_read_tokens"),
                    ("cache_creation_tokens", "cache_creation_tokens"),
                    ("input_tokens_include_cache", "input_tokens_include_cache"),
                    (
                        "input_tokens_include_cache_known",
                        "input_tokens_include_cache_known",
                    ),
                    ("total_tokens", "total_tokens"),
                    ("cost_known", "cost_known"),
                    ("latency_ms", "latency_ms"),
                    ("success", "success"),
                    ("retry_scheduled", "retry_scheduled"),
                    ("retry_after_ms", "retry_after_ms"),
                    ("fallback_scheduled", "fallback_scheduled"),
                    ("fallback_from_model", "fallback_from_model"),
                    ("fallback_to_model", "fallback_to_model"),
                    ("fallback_reason", "fallback_reason"),
                    ("failure_kind", "failure_kind"),
                    ("tool_count", "tool_count"),
                    ("input_item_count", "input_item_count"),
                    ("output_item_count", "output_item_count"),
                    ("instructions_length", "instructions_length"),
                ],
            );
            attrs.push(KeyValue::new("gen.cost_usd", g.cost_usd));
            attrs.push(KeyValue::new("gen.error", redact(&g.error)));
            "llm.generation".into()
        }
        SpanData::Handoff { .. } => {
            fields("handoff", &[("from", "from_agent"), ("to", "to_agent")]);
            "handoff".into()
        }
        SpanData::Guardrail { guardrail_name, .. } => {
            fields(
                "guardrail",
                &[("name", "guardrail_name"), ("triggered", "triggered")],
            );
            format!("guardrail.{guardrail_name}")
        }
        SpanData::Compaction { .. } => {
            fields(
                "compaction",
                &[
                    ("tokens_before", "tokens_before"),
                    ("tokens_after", "tokens_after"),
                ],
            );
            "compaction".into()
        }
        SpanData::Session(d) => {
            fields(
                "session",
                &[
                    ("model", "model"),
                    ("num_turns", "num_turns"),
                    ("duration_ms", "duration_ms"),
                    ("input_tokens", "input_tokens"),
                    ("output_tokens", "output_tokens"),
                    ("cache_read_tokens", "cache_read_input_tokens"),
                    ("cache_creation_tokens", "cache_creation_input_tokens"),
                    ("stop_reason", "stop_reason"),
                ],
            );
            attrs.push(KeyValue::new("session.cost_usd", d.cost_usd));
            "session".into()
        }
        SpanData::Subagent(d) => {
            fields(
                "subagent",
                &[
                    ("task_id", "task_id"),
                    ("type", "subagent_type"),
                    ("model", "model"),
                    ("status", "status"),
                    ("num_turns", "num_turns"),
                    ("total_tokens", "total_tokens"),
                    ("input_tokens", "input_tokens"),
                    ("output_tokens", "output_tokens"),
                    ("cache_read_tokens", "cache_read_tokens"),
                    ("cache_creation_tokens", "cache_creation_tokens"),
                    ("tool_count", "tool_count"),
                    ("duration_ms", "duration_ms"),
                    ("stop_reason", "stop_reason"),
                    ("isolation", "isolation"),
                ],
            );
            attrs.push(KeyValue::new("subagent.cost_usd", d.cost_usd));
            attrs.push(KeyValue::new(
                "subagent.description",
                redact(&truncate(&d.description, 200)),
            ));
            format!("subagent.{}", d.subagent_type)
        }
        SpanData::Retry {
            error_code,
            attempt,
            retry_after_ms,
            max_retries,
        } => {
            attrs.extend([
                KeyValue::new("retry.error_code", error_code.clone()),
                KeyValue::new("retry.attempt", *attempt),
                KeyValue::new("retry.after_ms", *retry_after_ms),
                KeyValue::new("retry.max_retries", *max_retries),
            ]);
            "api.retry".into()
        }
    };
    (name, attrs)
}

impl adk_runtime::tracing::GenerationObserver for SpanProcessor {
    fn start(&self, context: &adk_core::Context, record: &adk_runtime::tracing::GenerationRecord) {
        self.on_span_start(&generation_span(context, record));
    }
    fn end(&self, context: &adk_core::Context, record: &adk_runtime::tracing::GenerationRecord) {
        self.on_span_end(&generation_span(context, record));
    }
}
