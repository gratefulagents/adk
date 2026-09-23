//! SDK schema-2 trace producers. Lifecycle and capture policy are host-owned.

use crate::{
    observability::{CaptureMode, Redactor},
    tracestore::{RunMetadata, StoreError, TRACE_SCHEMA_VERSION, TraceStore},
};
use adk_core::{BoxFuture, Content, Context, Error, ModelResponse, RunItem};
use adk_runtime::{Observation, RunHooks};
use chrono::{DateTime, FixedOffset, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex},
    time::{Instant, SystemTime},
};

type Timestamp = DateTime<FixedOffset>;
fn now() -> Timestamp {
    DateTime::<Utc>::from(SystemTime::now()).fixed_offset()
}

#[derive(Clone, Default)]
pub struct Options {
    pub capture: CaptureMode,
    pub redactors: Vec<Redactor>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Health {
    pub events_written: u64,
    pub events_truncated: u64,
    pub events_dropped: u64,
    pub write_errors: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_error: String,
}

/// Serialized snapshot bytes retain field order for the SDK's metadata digest.
#[derive(Debug, Clone)]
pub struct Snapshot {
    bytes: Vec<u8>,
    value: Value,
}
impl Snapshot {
    pub fn from_json(bytes: Vec<u8>) -> Result<Self, serde_json::Error> {
        let value = serde_json::from_slice(&bytes)?;
        Ok(Self { bytes, value })
    }
    pub fn from_serializable(value: &impl Serialize) -> Result<Self, serde_json::Error> {
        Self::from_json(adk_codec::snapshots::to_go_json(value)?)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Generation {
    pub requested_model: String,
    pub resolved_model: String,
    pub model_provider: String,
    pub model_canonical: String,
    pub attempt_number: i64,
    pub generation_turn: i64,
    pub scope: String,
    pub task_id: String,
    pub status: String,
    pub usage_available: bool,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub input_tokens_include_cache: bool,
    pub input_tokens_include_cache_known: bool,
    pub total_tokens: i64,
    pub cost_usd: f64,
    pub cost_known: bool,
    pub latency_ms: i64,
    pub success: bool,
    pub retry_scheduled: bool,
    pub retry_after_ms: i64,
    pub fallback_scheduled: bool,
    pub fallback_from_model: String,
    pub fallback_to_model: String,
    pub fallback_reason: String,
    pub failure_kind: String,
    pub tool_count: i64,
    pub input_item_count: i64,
    pub output_item_count: i64,
    pub instructions_length: i64,
    pub input_token_estimate: i64,
    pub request_overhead_token_estimate: i64,
    pub total_request_token_estimate: i64,
    #[serde(skip)]
    pub request: Option<Snapshot>,
    #[serde(skip)]
    pub response: Option<Snapshot>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub model: String,
    pub cost_usd: f64,
    pub num_turns: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub stop_reason: String,
    pub duration_ms: i64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Subagent {
    pub task_id: String,
    pub subagent_type: String,
    pub description: String,
    pub model: String,
    pub status: String,
    pub cost_usd: f64,
    pub num_turns: i64,
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub tool_count: i64,
    pub duration_ms: i64,
    pub stop_reason: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub isolation: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub result_text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files_read: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files_written: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "span_type", rename_all = "snake_case")]
pub enum SpanData {
    Generation(Box<Generation>),
    Function {
        tool_name: String,
        #[serde(skip)]
        input: String,
        #[serde(skip)]
        output: String,
        is_error: bool,
    },
    Handoff {
        from_agent: String,
        to_agent: String,
    },
    Guardrail {
        guardrail_name: String,
        triggered: bool,
    },
    Compaction {
        tokens_before: i64,
        tokens_after: i64,
    },
    Session(Session),
    Subagent(Box<Subagent>),
    Agent {
        agent_name: String,
        #[serde(skip)]
        instructions: String,
    },
    Retry {
        error_code: String,
        attempt: i64,
        #[serde(skip)]
        retry_after_ms: i64,
        #[serde(skip)]
        max_retries: i64,
    },
}
#[derive(Debug, Clone)]
pub struct Span {
    pub id: String,
    pub parent_id: String,
    pub name: String,
    pub start_time: Timestamp,
    pub end_time: Timestamp,
    pub data: Option<SpanData>,
}
impl Span {
    pub fn new(
        name: impl Into<String>,
        parent_id: impl Into<String>,
        data: Option<SpanData>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            parent_id: parent_id.into(),
            name: name.into(),
            start_time: now(),
            end_time: crate::tracestore::ZERO_TIME
                .parse()
                .expect("zero timestamp"),
            data,
        }
    }
    pub fn finish(&mut self) {
        self.end_time = now();
    }
    pub fn duration_ms(&self) -> i64 {
        let end = if self.end_time
            == crate::tracestore::ZERO_TIME
                .parse::<Timestamp>()
                .expect("zero timestamp")
        {
            now()
        } else {
            self.end_time
        };
        (end - self.start_time)
            .num_nanoseconds()
            .unwrap_or(if end < self.start_time {
                i64::MIN
            } else {
                i64::MAX
            })
            / 1_000_000
    }
}

#[derive(Debug, Clone)]
pub struct Trace {
    pub spans: Vec<Span>,
    pub id: String,
    pub name: String,
    pub start_time: Timestamp,
    pub end_time: Timestamp,
}

impl Trace {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            start_time: now(),
            end_time: crate::tracestore::ZERO_TIME
                .parse()
                .expect("zero timestamp"),
            spans: vec![],
        }
    }
    pub fn add_span(&mut self, span: Span) {
        self.spans.push(span);
    }
    pub fn finish(&mut self) {
        self.end_time = now();
    }
}

struct State {
    store: Arc<dyn TraceStore>,
    run_id: String,
    options: Options,
    turn: u64,
    tools: HashMap<String, (Instant, String)>,
    models: HashMap<String, String>,
    health: Health,
}

fn digest(bytes: &[u8]) -> Value {
    json!({"captured":false, "sha256":format!("{:x}", Sha256::digest(bytes)), "bytes":bytes.len()})
}
impl State {
    fn redact(&self, text: &str) -> String {
        static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
            [r"(?i)(bearer\s+)[A-Za-z0-9._\-]+",
             r#"(?i)("(?:access_token|refresh_token|id_token|api_key|authorization|password|secret|token)"\s*:\s*")[^"]+(")"#,
             r#"(?i)(\\"(?:access_token|refresh_token|id_token|api_key|authorization|password|secret|token)\\"\s*:\s*\\")[^\\"]+(\\")"#,
             r"\bsk-[A-Za-z0-9_-]+", r"\bgh[pousr]_[A-Za-z0-9_]+\b"]
                .iter().map(|pattern| Regex::new(pattern).expect("static trace redactor")).collect()
        });
        let mut text = adk_security::redact_secrets(text).0;
        for pattern in PATTERNS.iter() {
            text = pattern
                .replace_all(&text, "${1}[REDACTED]${2}")
                .into_owned();
        }
        for redactor in &self.options.redactors {
            text = redactor(&text);
        }
        text
    }
    fn sanitize(&self, value: &Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.redact(text)),
            Value::Array(values) => {
                Value::Array(values.iter().map(|value| self.sanitize(value)).collect())
            }
            Value::Object(values) => Value::Object(
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), self.sanitize(value)))
                    .collect(),
            ),
            value => value.clone(),
        }
    }
    fn content(&self, value: &Value) -> Value {
        if self.options.capture == CaptureMode::Full {
            self.sanitize(value)
        } else if let Value::String(text) = value {
            digest(text.as_bytes())
        } else {
            digest(&adk_codec::snapshots::to_go_json(value).expect("serializable content"))
        }
    }
    fn raw_content(&self, bytes: &[u8]) -> Value {
        if self.options.capture == CaptureMode::Metadata {
            digest(bytes)
        } else {
            serde_json::from_slice::<Value>(bytes)
                .map(|value| self.sanitize(&value))
                .unwrap_or_else(|_| json!(self.redact(&String::from_utf8_lossy(bytes))))
        }
    }
    fn append(&mut self, category: &str, mut data: Value) {
        if self.run_id.is_empty() {
            return;
        }
        data["schema_version"] = json!(TRACE_SCHEMA_VERSION);
        data["run_id"] = json!(self.run_id);
        let bytes = adk_codec::snapshots::to_go_json(&data).expect("serializable trace record");
        let mut bytes = self
            .redact(std::str::from_utf8(&bytes).expect("JSON is UTF-8"))
            .into_bytes();
        if bytes.len() >= 1 << 20 {
            let marker = json!({"schema_version":TRACE_SCHEMA_VERSION, "run_id":self.run_id,
                "type":"event_truncated", "original_type":data["type"], "category":category,
                "sha256":format!("{:x}", Sha256::digest(&bytes)), "original_bytes":bytes.len(), "timestamp":now()});
            bytes =
                adk_codec::snapshots::to_go_json(&marker).expect("serializable truncation marker");
            self.health.events_truncated += 1;
        }
        match self.store.append_trace(&self.run_id, category, &bytes) {
            Ok(()) => self.health.events_written += 1,
            Err(error) => {
                if matches!(error, StoreError::EventTooLarge | StoreError::CategoryFull) {
                    self.health.events_dropped += 1;
                } else {
                    self.health.write_errors += 1;
                }
                self.health.last_error = format!("append {category}: {error}");
            }
        }
    }
    fn instructions(&mut self, path: &str, text: &str) {
        if self.run_id.is_empty() {
            return;
        }
        let bytes = if self.options.capture == CaptureMode::Full {
            self.redact(text).into_bytes()
        } else {
            serde_json::to_vec(&digest(text.as_bytes())).expect("serializable digest")
        };
        if let Err(error) = self.store.write_file(&self.run_id, path, &bytes) {
            self.health.write_errors += 1;
            self.health.last_error = format!("write resolved instructions: {error}");
        }
    }
    fn span_data(&self, data: &SpanData) -> Map<String, Value> {
        let mut value = serde_json::to_value(data).expect("serializable span");
        if let SpanData::Generation(generation) = data {
            value["input_tokens"] = json!(generation.prompt_tokens);
            value["output_tokens"] = json!(generation.completion_tokens);
            value["gen_duration_ms"] = json!(generation.latency_ms);
            let model = if generation.resolved_model.is_empty() {
                &generation.requested_model
            } else {
                &generation.resolved_model
            };
            if !model.is_empty() {
                value["model"] = json!(model);
            }
            for (key, snapshot) in [
                ("request", &generation.request),
                ("response", &generation.response),
            ] {
                if let Some(snapshot) = snapshot {
                    value[key] = self.raw_content(&snapshot.bytes);
                }
            }
            if !generation.error.is_empty() {
                value["error"] = json!(self.redact(&generation.error));
            }
        }
        if let SpanData::Subagent(subagent) = data {
            value["description"] = json!(self.redact(&subagent.description));
            for (key, paths) in [
                ("files_read", &subagent.files_read),
                ("files_written", &subagent.files_written),
            ] {
                if !paths.is_empty() {
                    value[format!("{key}_count")] = json!(paths.len());
                }
            }
            for key in ["prompt", "result_text", "files_read", "files_written"] {
                if let Some(content) = value.get(key).cloned() {
                    value[key] = self.content(&content);
                }
            }
        }
        value.as_object().expect("span data object").clone()
    }
    fn span(&mut self, span: &Span, ending: bool) {
        let mut entry = json!({"type":if ending {"span_end"} else {"span_start"}, "span_id":span.id, "parent_id":span.parent_id, "name":span.name});
        if ending {
            entry["start_time"] = json!(span.start_time);
            entry["end_time"] = json!(span.end_time);
            entry["duration_ms"] = json!(span.duration_ms());
        } else {
            entry["timestamp"] = json!(span.start_time);
        }
        if let Some(data) = &span.data {
            entry.as_object_mut().unwrap().extend(self.span_data(data));
        }
        self.append("spans", entry.clone());
        if let Some(SpanData::Generation(generation)) = &span.data {
            entry["type"] = json!(if ending {
                "generation_end"
            } else {
                "generation_start"
            });
            entry["timestamp"] = json!(if ending {
                span.end_time
            } else {
                span.start_time
            });
            self.append("llm_calls", entry);
            if !ending {
                if let Some(instructions) = generation
                    .request
                    .as_ref()
                    .and_then(|request| request.value.get("instructions"))
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                {
                    self.instructions(
                        &format!(
                            "resolved_instructions/turn_{:03}_attempt_{:03}.txt",
                            generation.generation_turn, generation.attempt_number
                        ),
                        instructions,
                    );
                }
            }
        }
    }
}

pub struct TraceWriter {
    state: Mutex<State>,
}
impl TraceWriter {
    pub fn new(store: Arc<dyn TraceStore>, run_id: impl Into<String>, options: Options) -> Self {
        Self {
            state: Mutex::new(State {
                store,
                run_id: run_id.into(),
                options,
                turn: 0,
                tools: HashMap::new(),
                models: HashMap::new(),
                health: Health::default(),
            }),
        }
    }
    pub fn init_run(&self, metadata: &RunMetadata) -> Result<PathBuf, StoreError> {
        let mut state = self.state.lock().expect("trace writer poisoned");
        state.run_id = metadata.run_id.clone();
        state.store.create_run_dir(&state.run_id, metadata)
    }
    pub fn health(&self) -> Health {
        self.state
            .lock()
            .expect("trace writer poisoned")
            .health
            .clone()
    }
    pub fn agent_start(&self, agent: &str) {
        let mut state = self.state.lock().expect("trace writer poisoned");
        state.turn += 1;
        let turn = state.turn;
        state.append(
            "agent_transitions",
            json!({"type":"agent_start", "agent":agent, "turn":turn, "timestamp":now()}),
        );
    }
    pub fn agent_end(&self, agent: &str) {
        let mut state = self.state.lock().expect("trace writer poisoned");
        let turn = state.turn;
        state.append(
            "agent_transitions",
            json!({"type":"agent_end", "agent":agent, "turn":turn, "timestamp":now()}),
        );
    }
    pub fn handoff(&self, from: &str, to: &str) {
        let mut state = self.state.lock().expect("trace writer poisoned");
        let turn = state.turn;
        state.append(
            "agent_transitions",
            json!({"type":"handoff", "from":from, "to":to, "turn":turn, "timestamp":now()}),
        );
    }
    pub fn tool_start(
        &self,
        agent: &str,
        tool: &str,
        call_id: &str,
        input: &[u8],
        parent_call_id: &str,
    ) {
        let mut state = self.state.lock().expect("trace writer poisoned");
        state
            .tools
            .insert(call_id.into(), (Instant::now(), agent.into()));
        let mut entry = json!({"type":"tool_start", "call_id":call_id, "tool":tool, "agent":agent, "input":state.raw_content(input), "turn":state.turn, "timestamp":now()});
        if !parent_call_id.is_empty() {
            entry["parent_call_id"] = json!(parent_call_id);
        }
        if matches!(tool, "Bash" | "ReadOnlyBash" | "WorkspaceWriteBash") {
            if let Ok(input) = serde_json::from_slice::<Value>(input) {
                if let Some(command) = input
                    .get("command")
                    .and_then(Value::as_str)
                    .filter(|command| !command.is_empty())
                {
                    entry["bash_command"] = state.content(&json!(command));
                }
            }
        }
        state.append("tool_calls", entry);
    }
    pub fn tool_end(
        &self,
        agent: &str,
        tool: &str,
        call_id: &str,
        output: &str,
        is_error: bool,
        parent_call_id: &str,
    ) {
        let mut state = self.state.lock().expect("trace writer poisoned");
        let started = state.tools.remove(call_id);
        let agent = if agent.is_empty() {
            started
                .as_ref()
                .map(|(_, agent)| agent.as_str())
                .unwrap_or("")
        } else {
            agent
        };
        let duration_ms = started
            .as_ref()
            .map(|(start, _)| start.elapsed().as_millis() as u64)
            .unwrap_or(0);
        let mut entry = json!({"type":"tool_end", "call_id":call_id, "tool":tool, "agent":agent, "output":state.content(&json!(output)), "is_error":is_error, "duration_ms":duration_ms, "turn":state.turn, "timestamp":now()});
        if !parent_call_id.is_empty() {
            entry["parent_call_id"] = json!(parent_call_id);
        }
        state.append("tool_calls", entry);
    }
    pub fn llm_start(&self, agent: &str, model: &str) {
        let mut state = self.state.lock().expect("trace writer poisoned");
        state.models.insert(agent.into(), model.into());
        let turn = state.turn;
        state.append("llm_calls", json!({"type":"llm_start", "agent":agent, "model":model, "turn":turn, "timestamp":now()}));
    }
    pub fn llm_end(&self, agent: &str, response: &ModelResponse) {
        let mut state = self.state.lock().expect("trace writer poisoned");
        let mut text_count = 0;
        let mut reasoning_count = 0;
        let mut calls = Vec::new();
        for item in &response.items {
            match item {
                RunItem::Message { message } | RunItem::PhasedMessage { message, .. }
                    if message.content.iter().any(
                        |content| matches!(content, Content::Text { text } if !text.is_empty()),
                    ) =>
                {
                    text_count += 1
                }
                RunItem::Reasoning { reasoning } if !reasoning.text.is_empty() => {
                    reasoning_count += 1
                }
                RunItem::ToolCall { call } => calls.push(json!({"id":call.id, "name":call.name})),
                _ => {}
            }
        }
        let mut entry = json!({"type":"llm_end", "agent":agent, "model":state.models.get(agent).map(String::as_str).unwrap_or(""), "text_count":text_count, "reasoning_count":reasoning_count,
            "tool_calls":if calls.is_empty() { Value::Null } else { json!(calls) }, "input_tokens":response.usage.input_tokens, "output_tokens":response.usage.output_tokens,
            "turn":state.turn, "timestamp":now(), "usage_populated":response.usage.input_tokens > 0 || response.usage.output_tokens > 0});
        if response.usage.cache_read_tokens > 0 {
            entry["cache_read_tokens"] = json!(response.usage.cache_read_tokens);
        }
        if response.usage.cache_creation_tokens > 0 {
            entry["cache_creation_tokens"] = json!(response.usage.cache_creation_tokens);
        }
        state.append("llm_calls", entry);
    }
    pub fn trace_start(&self, trace: &Trace) {
        self.state.lock().expect("trace writer poisoned").append("spans", json!({"type":"trace_start", "trace_id":trace.id, "trace_name":trace.name, "timestamp":trace.start_time}));
    }
    pub fn trace_end(&self, trace: &Trace) {
        self.state.lock().expect("trace writer poisoned").append("spans", json!({"type":"trace_end", "trace_id":trace.id, "trace_name":trace.name, "start_time":trace.start_time, "end_time":trace.end_time}));
    }
    pub fn span_start(&self, span: &Span) {
        self.state
            .lock()
            .expect("trace writer poisoned")
            .span(span, false);
    }
    pub fn span_end(&self, span: &Span) {
        self.state
            .lock()
            .expect("trace writer poisoned")
            .span(span, true);
    }
    pub fn write_resolved_instructions(&self, turn: i64, instructions: &str) {
        self.state
            .lock()
            .expect("trace writer poisoned")
            .instructions(
                &format!("resolved_instructions/turn_{turn:03}.txt"),
                instructions,
            );
    }
    pub fn record_phase_change(&self, phase: &str) {
        let mut state = self.state.lock().expect("trace writer poisoned");
        let turn = state.turn;
        state.append(
            "agent_transitions",
            json!({"type":"phase_change", "phase":phase, "turn":turn, "timestamp":now()}),
        );
    }
    pub fn record_mode_switch(&self, from: &str, to: &str) -> Result<(), StoreError> {
        let mut state = self.state.lock().expect("trace writer poisoned");
        let turn = state.turn;
        state.append("agent_transitions", json!({"type":"mode_switch", "from_mode":from, "to_mode":to, "turn":turn, "timestamp":now()}));
        state.store.update_metadata_mode(&state.run_id, to)
    }
    pub fn write_metrics(&self, metrics: &Map<String, Value>) -> Result<(), StoreError> {
        let state = self.state.lock().expect("trace writer poisoned");
        if state.run_id.is_empty() {
            return Ok(());
        }
        let bytes = state
            .redact(&serde_json::to_string_pretty(metrics)?)
            .into_bytes();
        state
            .store
            .write_file(&state.run_id, "metrics.json", &bytes)
    }
    pub fn finalize_run(&self, status: &str) -> Result<(), StoreError> {
        let mut state = self.state.lock().expect("trace writer poisoned");
        let turn = state.turn;
        state.append(
            "agent_transitions",
            json!({"type":"session_end", "status":status, "turn":turn, "timestamp":now()}),
        );
        let health = serde_json::to_vec_pretty(&state.health)?;
        state
            .store
            .write_file(&state.run_id, "trace_health.json", &health)?;
        state
            .store
            .update_metadata_finished_at(&state.run_id, now())
    }
}
impl RunHooks for TraceWriter {
    fn durable_observer(&self) -> bool {
        true
    }
    fn observe<'a>(
        &'a self,
        _context: &'a Context,
        observation: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            match observation {
                Observation::AgentStarted { agent } => self.agent_start(&agent),
                Observation::AgentEnded { agent, .. } => self.agent_end(&agent),
                Observation::Handoff { from, to } => self.handoff(&from, &to),
                Observation::ToolStarted { agent, call } => self.tool_start(
                    &agent,
                    &call.name,
                    &call.id,
                    &serde_json::to_vec(&call.arguments).expect("serializable arguments"),
                    "",
                ),
                Observation::RawToolOutput { call, output } => {
                    let text = output
                        .content
                        .iter()
                        .filter_map(|content| {
                            if let Content::Text { text } = content {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<String>();
                    self.tool_end("", &call.name, &call.id, &text, output.is_error, "");
                }
                Observation::ModelAttempt { agent, model, .. } => self.llm_start(&agent, &model),
                Observation::ModelAccepted { agent, response } => self.llm_end(&agent, &response),
                _ => {}
            }
            Ok(())
        })
    }
}

impl adk_runtime::tracing::GenerationObserver for TraceWriter {
    fn start(&self, context: &Context, record: &adk_runtime::tracing::GenerationRecord) {
        self.span_start(&generation_span(context, record));
    }
    fn end(&self, context: &Context, record: &adk_runtime::tracing::GenerationRecord) {
        let mut span = generation_span(context, record);
        if let (Some(response), Some(SpanData::Generation(data))) =
            (&record.response, &mut span.data)
        {
            let snapshot = adk_codec::dto::ResponseSnapshot::try_from(response)
                .map_err(|error| error.to_string())
                .and_then(|snapshot| {
                    Snapshot::from_serializable(&snapshot).map_err(|error| error.to_string())
                });
            match snapshot {
                Ok(snapshot) => data.response = Some(snapshot),
                Err(error) => {
                    self.state
                        .lock()
                        .expect("trace writer poisoned")
                        .health
                        .last_error = format!("snapshot response: {error}");
                }
            }
        }
        self.span_end(&span);
    }
}

pub(crate) fn generation_span(
    context: &adk_core::Context,
    record: &adk_runtime::tracing::GenerationRecord,
) -> Span {
    use crate::tracewriter::Generation;
    use adk_runtime::tracing::GenerationStatus;
    let model = record.request.model.trim();
    // Go lowercases one rune at a time, without expansion or final-sigma context.
    let mut provider: String = record
        .provider
        .trim()
        .chars()
        .map(|c| c.to_lowercase().next().unwrap())
        .collect();
    let (prefix, bare) = model.split_once('/').unwrap_or(("", model));
    let canonical = if bare.is_empty() {
        model.into()
    } else if !prefix.is_empty() {
        let prefix: String = prefix
            .chars()
            .map(|c| c.to_lowercase().next().unwrap())
            .collect();
        if provider.is_empty() {
            provider.clone_from(&prefix);
        }
        format!("{prefix}/{bare}")
    } else if !provider.is_empty() {
        format!("{provider}/{bare}")
    } else {
        model.into()
    };
    let error = record
        .error
        .as_ref()
        .map_or("", |error| error.message.as_str());
    let failure_kind = record.retry_reason.clone().unwrap_or_else(|| {
        record.error.as_ref().map_or_else(String::new, |error| {
            if error
                .message
                .to_ascii_lowercase()
                .contains("context_length_exceeded")
                || error
                    .message
                    .to_ascii_lowercase()
                    .contains("exceeds the context window")
            {
                return "context_length_exceeded".into();
            }
            match error.category {
                adk_core::ErrorCategory::Cancelled => "context_canceled",
                adk_core::ErrorCategory::DeadlineExceeded => "deadline_exceeded",
                adk_core::ErrorCategory::ModelBehavior => "model_behavior",
                _ => "error",
            }
            .into()
        })
    });
    let mut generation = Generation {
        requested_model: model.into(),
        resolved_model: record.resolved_model.clone(),
        input_tokens_include_cache: record.input_tokens_include_cache.unwrap_or(false),
        input_tokens_include_cache_known: record.input_tokens_include_cache.is_some(),
        model_provider: provider,
        model_canonical: canonical,
        attempt_number: i64::from(record.turn),
        generation_turn: i64::from(record.turn),
        scope: if record.task_id.is_some() {
            "subagent"
        } else {
            "top_level"
        }
        .into(),
        task_id: record.task_id.clone().unwrap_or_default(),
        status: match record.status {
            GenerationStatus::Started => "",
            GenerationStatus::Completed => "completed",
            GenerationStatus::Failed => "failed",
            GenerationStatus::Retrying => "retrying",
            GenerationStatus::Fallback => "fallback",
            GenerationStatus::Interrupted => "interrupted",
        }
        .into(),
        latency_ms: record.latency.as_millis().min(i64::MAX as u128) as i64,
        success: record.status == GenerationStatus::Completed,
        retry_scheduled: record.retry_after.is_some(),
        retry_after_ms: record
            .retry_after
            .map_or(0, |delay| delay.as_millis().min(i64::MAX as u128) as i64),
        fallback_scheduled: record.fallback_model.is_some(),
        fallback_from_model: record
            .fallback_model
            .as_ref()
            .map_or_else(String::new, |_| model.into()),
        fallback_to_model: record.fallback_model.clone().unwrap_or_default(),
        failure_kind: failure_kind.clone(),
        error: error.into(),
        tool_count: record.request.tools.len() as i64,
        input_item_count: record.request.input.len() as i64,
        instructions_length: record.request.instructions.len() as i64,
        cost_usd: record.cost_usd.unwrap_or_default(),
        cost_known: record.cost_usd.is_some(),
        ..Default::default()
    };
    if record.fallback_model.is_some() {
        let reason = record
            .retry_reason
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase();
        generation.fallback_reason = if reason == "429"
            || ["rate_limit", "too_many_requests", "too many requests"]
                .iter()
                .any(|s| reason.contains(s))
        {
            "rate_limit".into()
        } else if matches!(reason.as_str(), "503" | "529") || reason.contains("overloaded") {
            "overloaded".into()
        } else if reason == "402"
            || [
                "quota",
                "billing",
                "subscription",
                "credit",
                "limit_exceeded",
                "exhausted",
            ]
            .iter()
            .any(|s| reason.contains(s))
        {
            "quota".into()
        } else {
            failure_kind
        };
    }
    if let Some(response) = &record.response {
        generation.usage_available = true;
        generation.prompt_tokens = response.usage.input_tokens.min(i64::MAX as u64) as i64;
        generation.completion_tokens = response.usage.output_tokens.min(i64::MAX as u64) as i64;
        generation.cache_read_tokens = response.usage.cache_read_tokens.min(i64::MAX as u64) as i64;
        generation.cache_creation_tokens =
            response.usage.cache_creation_tokens.min(i64::MAX as u64) as i64;
        generation.total_tokens = response
            .usage
            .input_tokens
            .saturating_add(response.usage.output_tokens)
            .min(i64::MAX as u64) as i64;
        generation.output_item_count = response.items.len() as i64;
    }
    Span {
        id: record.id.clone(),
        parent_id: context.run_id.clone(),
        name: "generation".into(),
        start_time: chrono::DateTime::<chrono::Utc>::from(record.started_at).fixed_offset(),
        end_time: record
            .ended_at
            .map(|end| chrono::DateTime::<chrono::Utc>::from(end).fixed_offset())
            .unwrap_or_else(|| {
                crate::tracestore::ZERO_TIME
                    .parse()
                    .expect("zero timestamp")
            }),
        data: Some(SpanData::Generation(Box::new(generation))),
    }
}
