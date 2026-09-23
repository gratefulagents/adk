//! Ordered native observations, explicit capture policy, private traces and an
//! optional real OpenTelemetry tracer bridge. See `docs/observability.md`.

use adk_core::{
    ApprovalDecision, ApprovalRequest, BoxFuture, Context, Error, ErrorCategory, Host, ModelEvent,
    RunEvent, Usage,
};
use adk_runtime::{Observation, RunHooks};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

/// Native schema, deliberately distinct from Go TraceSchemaVersion 2.
pub const EVENT_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_MAX_EVENT_BYTES: usize = 1024 * 1024;

fn invalid(message: &'static str) -> Error {
    Error::new(ErrorCategory::InvalidInput, message)
}
fn host_error(message: &'static str) -> Error {
    Error::new(ErrorCategory::Host, message)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProgressSnapshot {
    pub sequence: u64,
    pub agent_turns: u64,
    pub model_attempts: u64,
    pub tool_calls: u64,
    pub tool_results: u64,
    pub retries: u64,
    pub handoffs: u64,
    pub compactions: u64,
    pub usage: Usage,
    /// Host CostEstimator units; not necessarily USD.
    pub cost: f64,
    pub recent: VecDeque<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    pub schema_version: u32,
    pub run_id: String,
    pub sequence: u64,
    pub timestamp_unix_ms: u64,
    pub kind: String,
    pub data: Value,
    pub progress: ProgressSnapshot,
}

impl EventRecord {
    pub fn to_json_line(&self) -> Result<Vec<u8>, Error> {
        let mut bytes = serde_json::to_vec(self)
            .map_err(|error| host_error("encode observation").with_source(error))?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn from_json_line(bytes: &[u8]) -> Result<Self, Error> {
        let record: Self = serde_json::from_slice(bytes)
            .map_err(|error| invalid("invalid observation JSON").with_source(error))?;
        if record.schema_version != EVENT_SCHEMA_VERSION {
            return Err(invalid("unsupported native observation schema"));
        }
        if record.sequence == 0 || record.run_id.is_empty() {
            return Err(invalid(
                "observation requires run identity and positive sequence",
            ));
        }
        Ok(record)
    }
}

/// Bounded incremental JSONL decoder. Errors belong to individual lines; an
/// oversized line is discarded through its delimiter before decoding resumes.
pub struct LineDecoder {
    buffer: Vec<u8>,
    max_bytes: usize,
    discarding: bool,
}
impl LineDecoder {
    pub fn new(max_bytes: usize) -> Result<Self, Error> {
        if max_bytes == 0 {
            return Err(invalid("line limit must be positive"));
        }
        Ok(Self {
            buffer: Vec::new(),
            max_bytes,
            discarding: false,
        })
    }
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<EventRecord, Error>> {
        let mut records = Vec::new();
        for byte in bytes {
            if *byte == b'\n' {
                if !self.discarding && !self.buffer.iter().all(u8::is_ascii_whitespace) {
                    records.push(EventRecord::from_json_line(&self.buffer));
                }
                self.buffer.clear();
                self.discarding = false;
            } else if !self.discarding {
                if self.buffer.len() == self.max_bytes {
                    self.buffer.clear();
                    self.discarding = true;
                    records.push(Err(invalid("observation line exceeds byte limit")));
                } else {
                    self.buffer.push(*byte);
                }
            }
        }
        records
    }
    /// Explicit EOF, including a final record without a newline.
    pub fn finish(&mut self) -> Option<Result<EventRecord, Error>> {
        let result = if self.discarding || self.buffer.iter().all(u8::is_ascii_whitespace) {
            None
        } else {
            Some(EventRecord::from_json_line(&self.buffer))
        };
        self.buffer.clear();
        self.discarding = false;
        result
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CaptureMode {
    #[default]
    Metadata,
    /// Explicit high-trust opt-in, still subject to best-effort redaction.
    Full,
}
pub type Redactor = Arc<dyn Fn(&str) -> String + Send + Sync>;

#[derive(Clone, Default)]
pub struct CapturePolicy {
    pub mode: CaptureMode,
    pub redactors: Vec<Redactor>,
}
impl CapturePolicy {
    fn text(&self, text: &str) -> String {
        if adk_security::check_secrets(text).is_err() {
            return "[REDACTED]".into();
        }
        let mut text = text.to_owned();
        for redactor in &self.redactors {
            text = redactor(&text);
        }
        if adk_security::check_secrets(&text).is_err() {
            "[REDACTED]".into()
        } else {
            text
        }
    }
    pub fn capture(&self, data: &Value) -> Value {
        self.value(None, data)
    }
    fn value(&self, key: Option<&str>, value: &Value) -> Value {
        let key = key.unwrap_or("");
        if matches!(
            key.to_ascii_lowercase().as_str(),
            "authorization"
                | "password"
                | "secret"
                | "token"
                | "api_key"
                | "access_token"
                | "refresh_token"
                | "id_token"
        ) {
            return json!("[REDACTED]");
        }
        let content = matches!(
            key,
            "input"
                | "output"
                | "arguments"
                | "content"
                | "text"
                | "delta"
                | "message"
                | "reason"
                | "response"
                | "result"
                | "items"
                | "history"
                | "before"
                | "after"
                | "markers"
                | "metadata"
                | "payload"
                | "error"
        );
        let identifier = matches!(
            key,
            "type"
                | "agent"
                | "from"
                | "to"
                | "model"
                | "call_id"
                | "parent_call_id"
                | "task_id"
                | "tool_name"
                | "category"
                | "decision"
                | "phase"
                | "status"
        );
        let metric = matches!(
            key,
            "usage"
                | "requests"
                | "input_tokens"
                | "output_tokens"
                | "cache_read_tokens"
                | "cache_creation_tokens"
                | "context_tokens"
                | "target_tokens"
                | "cost"
                | "attempt"
                | "delay_ms"
                | "before_items"
                | "after_items"
                | "new_items_before"
                | "history_before"
                | "is_error"
                | "should_pause"
        );
        if self.mode == CaptureMode::Metadata
            && (content
                || (key.is_empty() && !value.is_object())
                || (!key.is_empty() && !identifier && !metric))
        {
            let bytes = if let Value::String(text) = value {
                text.as_bytes().to_vec()
            } else {
                serde_json::to_vec(value).expect("JSON value is serializable")
            };
            return json!({"sha256": format!("{:x}", Sha256::digest(&bytes)), "bytes": bytes.len()});
        }
        match value {
            Value::String(text) => {
                // Tool output commonly carries JSON as text rather than as an object.
                if let Ok(decoded @ (Value::Object(_) | Value::Array(_))) =
                    serde_json::from_str::<Value>(text)
                {
                    Value::String(self.value(None, &decoded).to_string())
                } else {
                    Value::String(self.text(text))
                }
            }
            Value::Array(values) => {
                Value::Array(values.iter().map(|v| self.value(None, v)).collect())
            }
            Value::Object(values) => Value::Object(
                values
                    .iter()
                    .map(|(k, v)| (self.text(k), self.value(Some(k), v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

/// Sinks see only capture-policy-processed records. Delivery is awaited and
/// fail-closed; implementations must not recursively publish to their pipeline.
pub trait EventSink: Send + Sync {
    fn emit<'a>(&'a self, record: &'a EventRecord) -> BoxFuture<'a, Result<(), Error>>;
    /// Finalize only this run; other pipelines may share the sink.
    fn finish_run<'a>(&'a self, _run_id: &'a str) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    /// Sink-owner shutdown, never invoked by an individual run pipeline.
    fn shutdown(&self) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraceHealth {
    pub events_attempted: u64,
    pub events_written: u64,
    pub write_errors: u64,
    pub last_error: Option<String>,
}
#[derive(Default)]
struct State {
    progress: ProgressSnapshot,
    health: TraceHealth,
    closed: bool,
    terminal: bool,
}

/// One run, one total order, shared by both RunHooks and ObservedHost. Delivery
/// failures consume a sequence number; already delivered sinks are not rolled back.
pub struct Observability {
    run_id: String,
    capture: CapturePolicy,
    sinks: Vec<Arc<dyn EventSink>>,
    state: Mutex<State>,
}
impl Observability {
    pub fn new(
        run_id: impl Into<String>,
        capture: CapturePolicy,
        sinks: Vec<Arc<dyn EventSink>>,
    ) -> Result<Self, Error> {
        let run_id = run_id.into();
        if run_id.is_empty() {
            return Err(invalid("run ID must not be empty"));
        }
        Ok(Self {
            run_id,
            capture,
            sinks,
            state: Mutex::new(State::default()),
        })
    }
    pub async fn snapshot(&self) -> ProgressSnapshot {
        self.state.lock().await.progress.clone()
    }
    pub async fn health(&self) -> TraceHealth {
        self.state.lock().await.health.clone()
    }

    pub async fn publish(&self, context: &Context, kind: &str, data: Value) -> Result<(), Error> {
        if context.run_id != self.run_id {
            return Err(invalid("observation belongs to another run"));
        }
        let usage = if kind == "usage" {
            Some((
                serde_json::from_value::<Usage>(data["usage"].clone())
                    .map_err(|error| invalid("invalid usage observation").with_source(error))?,
                data["cost"]
                    .as_f64()
                    .filter(|cost| cost.is_finite())
                    .ok_or_else(|| invalid("invalid observation cost"))?,
            ))
        } else {
            None
        };
        let mut state = self.state.lock().await;
        if state.closed || state.terminal {
            return Err(host_error("observation pipeline is closed"));
        }
        state.progress.sequence += 1;
        let p = &mut state.progress;
        match kind {
            "agent_start" => p.agent_turns += 1,
            "model_attempt" => p.model_attempts += 1,
            "tool_start" => p.tool_calls += 1,
            "raw_tool_output" => p.tool_results += 1,
            "retry" => p.retries += 1,
            "handoff" => p.handoffs += 1,
            "compacted" => p.compactions += 1,
            "usage" => {
                let (usage, cost) = usage.expect("usage validated before publication");
                p.usage = usage;
                p.cost = cost;
            }
            _ => {}
        }
        if p.recent.len() == 20 {
            p.recent.pop_front();
        }
        p.recent.push_back(self.capture.text(kind));
        let record = EventRecord {
            schema_version: EVENT_SCHEMA_VERSION,
            run_id: self.run_id.clone(),
            sequence: p.sequence,
            timestamp_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            kind: self.capture.text(kind),
            data: self.capture.capture(&data),
            progress: p.clone(),
        };
        state.health.events_attempted += 1;
        for sink in &self.sinks {
            if let Err(error) = sink.emit(&record).await {
                state.health.write_errors += 1;
                state.health.last_error = Some("observation sink failed".into());
                return Err(host_error("observation sink failed").with_source(error));
            }
        }
        state.health.events_written += 1;
        state.terminal = kind == "error" || (kind == "done" && data["status"] != "paused");
        Ok(())
    }

    pub async fn record_run_event(&self, context: &Context, event: &RunEvent) -> Result<(), Error> {
        let (kind, data) = match event {
            RunEvent::Started { agent } => ("run_start", json!({"agent":agent})),
            RunEvent::Model { event } => match event {
                ModelEvent::TextDelta { delta } => ("delta", json!({"delta":delta})),
                ModelEvent::ReasoningDelta { delta } => ("reasoning_delta", json!({"delta":delta})),
                ModelEvent::ToolArgumentsDelta { call_id, delta } => (
                    "tool_arguments_delta",
                    json!({"call_id":call_id,"delta":delta}),
                ),
                ModelEvent::ItemDone { item } => ("item", json!({"items":[item]})),
                ModelEvent::Complete { response } => {
                    ("model_complete", json!({"response":response}))
                }
            },
            RunEvent::ToolStarted { call } => (
                "host_tool_start",
                json!({"call_id":call.id,"tool_name":call.name,"input":call.arguments}),
            ),
            RunEvent::ToolFinished { call_id, output } => (
                "host_tool_end",
                json!({"call_id":call_id,"output":output,"is_error":output.is_error}),
            ),
            RunEvent::ApprovalRequired { request } => (
                "approval_required",
                json!({"call_id":request.call.id,"tool_name":request.call.name,"input":request.call.arguments,"reason":request.reason}),
            ),
            RunEvent::Finished { result } => {
                ("done", json!({"result":result,"status":result.status}))
            }
            RunEvent::Failed { error } => (
                "error",
                json!({"category":error.category,"error":error.message}),
            ),
        };
        self.publish(context, kind, data).await
    }

    pub async fn shutdown(&self) -> Result<(), Error> {
        let mut state = self.state.lock().await;
        if state.closed {
            return Ok(());
        }
        state.closed = true;
        let mut failure = None;
        for sink in &self.sinks {
            if let Err(error) = sink.finish_run(&self.run_id).await {
                failure = Some(error);
            }
        }
        if let Some(error) = failure {
            state.health.write_errors += 1;
            state.health.last_error = Some("observation shutdown failed".into());
            Err(host_error("observation shutdown failed").with_source(error))
        } else {
            Ok(())
        }
    }
}
impl RunHooks for Observability {
    fn durable_observer(&self) -> bool {
        true
    }
    fn observe<'a>(
        &'a self,
        context: &'a Context,
        observation: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let (kind, data) = match observation {
                Observation::TextDelta { delta } => ("hook_delta", json!({"delta":delta})),
                Observation::CommittedItems {
                    items,
                    agents,
                    markers,
                } => (
                    "committed_items",
                    json!({"items":items,"agents":agents,"markers":markers.iter().map(|m| json!({"before_item":m.before_item,"marker":m.marker})).collect::<Vec<_>>()}),
                ),
                Observation::ApprovalMarker {
                    agent,
                    call,
                    decision,
                    reason,
                    new_items_before,
                    history_before,
                } => (
                    "approval_marker",
                    json!({"agent":agent,"call_id":call.id,"tool_name":call.name,"input":call.arguments,"decision":format!("{decision:?}"),"reason":reason,"new_items_before":new_items_before,"history_before":history_before}),
                ),
                Observation::HistoryReplaced { before, after } => {
                    ("history_replaced", json!({"before":before,"after":after}))
                }
                Observation::ApprovalHistoryReplaced {
                    before,
                    after,
                    markers,
                } => (
                    "approval_history_replaced",
                    json!({"before":before,"after":after,"markers":markers.iter().map(|m| json!({"before_item":m.before_item,"marker":m.marker})).collect::<Vec<_>>()}),
                ),
                Observation::AgentStarted { agent } => ("agent_start", json!({"agent":agent})),
                Observation::AgentEnded { agent, output } => {
                    ("agent_end", json!({"agent":agent,"output":output}))
                }
                Observation::ModelAccepted { agent, response } => (
                    "model_accepted",
                    json!({"agent":agent,"usage":response.usage,"response":response}),
                ),
                Observation::ToolStarted { agent, call } => (
                    "tool_start",
                    json!({"agent":agent,"call_id":call.id,"tool_name":call.name,"input":call.arguments}),
                ),
                Observation::ModelAttempt {
                    agent,
                    model,
                    attempt,
                } => (
                    "model_attempt",
                    json!({"agent":agent,"model":model,"attempt":attempt}),
                ),
                Observation::Retry { model, delay } => (
                    "retry",
                    json!({"model":model,"delay_ms":delay.as_millis() as u64}),
                ),
                Observation::Fallback { from, to } => ("fallback", json!({"from":from,"to":to})),
                Observation::RawToolOutput { call, output } => (
                    "raw_tool_output",
                    json!({"call_id":call.id,"tool_name":call.name,"is_error":output.is_error,"output":output}),
                ),
                Observation::Handoff { from, to } => ("handoff", json!({"from":from,"to":to})),
                Observation::CompactionStarted {
                    context_tokens,
                    target_tokens,
                } => (
                    "compaction_start",
                    json!({"context_tokens":context_tokens,"target_tokens":target_tokens}),
                ),
                Observation::CompactionFailed { error } => (
                    "compaction_error",
                    json!({"category":error.category,"error":error.message}),
                ),
                Observation::Compacted {
                    before_items,
                    after_items,
                    context_tokens,
                } => (
                    "compacted",
                    json!({"before_items":before_items,"after_items":after_items,"context_tokens":context_tokens}),
                ),
                Observation::OutputValidationFailed { message } => {
                    ("output_validation_failed", json!({"message":message}))
                }
                Observation::Usage { usage, cost } => ("usage", json!({"usage":usage,"cost":cost})),
            };
            self.publish(context, kind, data).await
        })
    }
}

/// Approval decisions and original host events are delegated without redaction.
/// Only the observability sinks receive capture-policy-processed records.
pub struct ObservedHost {
    pub host: Arc<dyn Host>,
    pub observations: Arc<Observability>,
}
impl Host for ObservedHost {
    fn emit<'a>(
        &'a self,
        context: &'a Context,
        event: RunEvent,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.observations.record_run_event(context, &event).await?;
            self.host.emit(context, event).await
        })
    }
    fn approve<'a>(
        &'a self,
        context: &'a Context,
        request: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        self.host.approve(context, request)
    }
}

#[derive(Debug, Clone)]
pub struct TraceLimits {
    pub event_bytes: usize,
    pub chunk_bytes: u64,
    /// Number of additional chunks after events.jsonl.
    pub rotations: u32,
}
impl Default for TraceLimits {
    fn default() -> Self {
        Self {
            event_bytes: DEFAULT_MAX_EVENT_BYTES,
            chunk_bytes: 64 * 1024 * 1024,
            rotations: 4,
        }
    }
}

#[cfg(unix)]
mod private_store {
    use super::*;
    use rustix::fs::{Mode, OFlags, fchmod, mkdirat, openat};
    use std::{
        fs::File,
        io::{self, Write},
        os::unix::fs::MetadataExt,
        path::{Component, Path},
        sync::Mutex as StdMutex,
    };

    fn directory(parent: &File, name: &std::ffi::OsStr, create: bool) -> io::Result<File> {
        if create {
            match mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
                Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(File::from(openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?))
    }
    fn private(file: &File, mode: u32) -> io::Result<()> {
        let metadata = file.metadata()?;
        if metadata.mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "trace path must be owner-private",
            ));
        }
        fchmod(file, Mode::from_raw_mode(mode))?;
        Ok(())
    }
    fn chunk(directory: &File, index: u32) -> io::Result<File> {
        let name = if index == 0 {
            "events.jsonl".into()
        } else {
            format!("events.jsonl.{index:03}")
        };
        let file = File::from(openat(
            directory,
            name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )?);
        private(&file, 0o600)?;
        Ok(file)
    }
    struct Writer {
        file: File,
        bytes: u64,
        index: u32,
        failed: bool,
        sequence: u64,
    }

    /// New run only: no reopening or overwriting existing runs. All components
    /// are walked through directory descriptors without following symlinks.
    pub struct FilesystemTraceStore {
        directory: File,
        run_id: String,
        limits: TraceLimits,
        writer: StdMutex<Writer>,
    }
    impl FilesystemTraceStore {
        pub fn create(
            root: impl AsRef<Path>,
            run_id: &str,
            limits: TraceLimits,
        ) -> io::Result<Self> {
            if run_id.is_empty()
                || run_id == "."
                || run_id == ".."
                || run_id.len() > 128
                || !run_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid trace run ID",
                ));
            }
            if limits.event_bytes == 0 || limits.chunk_bytes == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "trace limits must be positive",
                ));
            }
            let root = root.as_ref();
            if root.as_os_str().is_empty() || root == Path::new("/") || root == Path::new(".") {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "explicit private trace root required",
                ));
            }
            let mut fd = File::open(if root.is_absolute() { "/" } else { "." })?;
            for component in root.components() {
                match component {
                    Component::RootDir | Component::CurDir => {}
                    Component::Normal(name) => fd = directory(&fd, name, true)?,
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "trace root cannot contain parent traversal",
                        ));
                    }
                }
            }
            private(&fd, 0o700)?;
            let traces = directory(&fd, std::ffi::OsStr::new("traces"), true)?;
            private(&traces, 0o700)?;
            mkdirat(&traces, run_id, Mode::from_raw_mode(0o700))?;
            let directory = directory(&traces, std::ffi::OsStr::new(run_id), false)?;
            private(&directory, 0o700)?;
            let file = chunk(&directory, 0)?;
            Ok(Self {
                directory,
                run_id: run_id.into(),
                limits,
                writer: StdMutex::new(Writer {
                    file,
                    bytes: 0,
                    index: 0,
                    failed: false,
                    sequence: 0,
                }),
            })
        }
        pub fn append(&self, record: &EventRecord) -> Result<(), Error> {
            if record.run_id != self.run_id {
                return Err(invalid("trace record belongs to another run"));
            }
            let bytes = record.to_json_line()?;
            if bytes.len() > self.limits.event_bytes || bytes.len() as u64 > self.limits.chunk_bytes
            {
                return Err(host_error("trace event exceeds byte quota"));
            }
            let mut writer = self
                .writer
                .lock()
                .map_err(|_| host_error("trace writer poisoned"))?;
            if writer.failed {
                return Err(host_error("trace writer failed; refusing further appends"));
            }
            if record.sequence <= writer.sequence {
                return Err(invalid("trace sequences must increase"));
            }
            if writer.bytes + bytes.len() as u64 > self.limits.chunk_bytes {
                if writer.index == self.limits.rotations {
                    return Err(host_error("trace rotation quota exhausted"));
                }
                let next = chunk(&self.directory, writer.index + 1)
                    .map_err(|error| host_error("create trace chunk").with_source(error))?;
                writer.file = next;
                writer.index += 1;
                writer.bytes = 0;
            }
            if let Err(error) = writer
                .file
                .write_all(&bytes)
                .and_then(|_| writer.file.sync_data())
            {
                // A short write may have left a partial JSON line. Never append to it.
                writer.failed = true;
                return Err(host_error("persist trace record").with_source(error));
            }
            writer.bytes += bytes.len() as u64;
            writer.sequence = record.sequence;
            Ok(())
        }
    }
    impl EventSink for FilesystemTraceStore {
        fn emit<'a>(&'a self, record: &'a EventRecord) -> BoxFuture<'a, Result<(), Error>> {
            Box::pin(async move { self.append(record) })
        }
    }
}
#[cfg(unix)]
pub use private_store::FilesystemTraceStore;

/// Real OpenTelemetry API bridge; the host owns its SDK provider and exporters.
#[cfg(feature = "otel")]
pub mod otel {
    use super::*;
    use opentelemetry::{
        Context as OtelContext, KeyValue,
        trace::{SpanKind, Status, TraceContextExt, Tracer},
    };
    use std::{collections::HashMap, sync::Mutex as StdMutex};

    struct RunTrace {
        root: OtelContext,
        agent: Option<OtelContext>,
        agent_name: Option<String>,
        model: Option<OtelContext>,
        tools: HashMap<String, OtelContext>,
        ended_tools: HashMap<String, OtelContext>,
    }
    fn end(context: OtelContext, error: bool) {
        if error {
            context
                .span()
                .set_status(Status::error("agent operation failed or incomplete"));
        }
        context.span().end();
    }
    impl RunTrace {
        fn finish(mut self, error: bool) {
            for (_, span) in self.tools.drain() {
                end(span, true);
            }
            if let Some(span) = self.model.take() {
                end(span, true);
            }
            if let Some(span) = self.agent.take() {
                end(span, error);
            }
            end(self.root, error);
        }
    }

    pub struct OtelBridge<T: Tracer> {
        tracer: T,
        parent: OtelContext,
        runs: StdMutex<HashMap<String, RunTrace>>,
    }
    impl<T: Tracer + Send + Sync> OtelBridge<T>
    where
        T::Span: Send + Sync + 'static,
    {
        pub fn new(tracer: T, parent: OtelContext) -> Self {
            Self {
                tracer,
                parent,
                runs: StdMutex::new(HashMap::new()),
            }
        }
        fn start(
            &self,
            name: &'static str,
            parent: &OtelContext,
            attributes: Vec<KeyValue>,
            kind: SpanKind,
        ) -> OtelContext {
            let span = self.tracer.build_with_context(
                self.tracer
                    .span_builder(name)
                    .with_kind(kind)
                    .with_attributes(attributes),
                parent,
            );
            parent.with_span(span)
        }
        fn record(&self, record: &EventRecord) -> Result<(), Error> {
            let mut runs = self
                .runs
                .lock()
                .map_err(|_| host_error("telemetry bridge poisoned"))?;
            let run = runs
                .entry(record.run_id.clone())
                .or_insert_with(|| RunTrace {
                    root: self.start(
                        "adk.run",
                        &self.parent,
                        vec![KeyValue::new("adk.run.id", record.run_id.clone())],
                        SpanKind::Internal,
                    ),
                    agent: None,
                    agent_name: None,
                    model: None,
                    tools: HashMap::new(),
                    ended_tools: HashMap::new(),
                });
            let data = &record.data;
            let text = |key: &str| data[key].as_str().unwrap_or("").to_owned();
            let attributes = vec![
                KeyValue::new("adk.sequence", record.sequence as i64),
                KeyValue::new("adk.event.data", data.to_string()),
            ];
            run.root.span().add_event(record.kind.clone(), attributes);
            match record.kind.as_str() {
                "agent_start" => {
                    let name = text("agent");
                    if run.agent.is_some() && run.agent_name.as_ref() == Some(&name) {
                        return Ok(());
                    }
                    if let Some(previous) = run.agent.take() {
                        end(previous, false);
                    }
                    run.agent_name = Some(name.clone());
                    run.agent = Some(self.start(
                        "adk.agent",
                        &run.root,
                        vec![KeyValue::new("agent.name", name)],
                        SpanKind::Internal,
                    ));
                }
                "agent_end" => {
                    run.agent_name = None;
                    if let Some(span) = run.agent.take() {
                        end(span, false);
                    }
                }
                "model_attempt" => {
                    if let Some(previous) = run.model.take() {
                        end(previous, true);
                    }
                    run.model = Some(self.start(
                        "gen_ai.chat",
                        run.agent.as_ref().unwrap_or(&run.root),
                        vec![
                            KeyValue::new("gen_ai.request.model", text("model")),
                            KeyValue::new(
                                "gen.attempt_number",
                                data["attempt"].as_u64().unwrap_or(0) as i64,
                            ),
                        ],
                        SpanKind::Client,
                    ));
                }
                "model_accepted" => {
                    if let Some(span) = run.model.take() {
                        span.span().set_attribute(KeyValue::new(
                            "gen_ai.usage.input_tokens",
                            data["usage"]["input_tokens"].as_u64().unwrap_or(0) as i64,
                        ));
                        span.span().set_attribute(KeyValue::new(
                            "gen_ai.usage.output_tokens",
                            data["usage"]["output_tokens"].as_u64().unwrap_or(0) as i64,
                        ));
                        end(span, false);
                    }
                }
                "retry" | "fallback" => {
                    if let Some(span) = run.model.take() {
                        end(span, true);
                    }
                }
                "tool_start" => {
                    let id = text("call_id");
                    let parent = data["parent_call_id"]
                        .as_str()
                        .and_then(|id| run.tools.get(id).or_else(|| run.ended_tools.get(id)))
                        .or(run.agent.as_ref())
                        .unwrap_or(&run.root);
                    let span = self.start(
                        "adk.tool",
                        parent,
                        vec![
                            KeyValue::new("tool.name", text("tool_name")),
                            KeyValue::new("tool.call.id", id.clone()),
                            KeyValue::new("tool.input", data["input"].to_string()),
                        ],
                        SpanKind::Internal,
                    );
                    if let Some(previous) = run.tools.insert(id, span) {
                        end(previous, true);
                    }
                }
                "raw_tool_output" => {
                    if let Some(span) = run.tools.remove(&text("call_id")) {
                        let failed = data["is_error"].as_bool().unwrap_or(false);
                        span.span().set_attribute(KeyValue::new(
                            "tool.output",
                            data["output"].to_string(),
                        ));
                        span.span()
                            .set_attribute(KeyValue::new("tool.error", failed));
                        end(span.clone(), failed);
                        run.ended_tools.insert(text("call_id"), span);
                    }
                }
                "done" | "error" => {
                    if record.kind == "done" && data["status"] == "paused" {
                        run.root
                            .span()
                            .set_attribute(KeyValue::new("adk.status", "paused"));
                        return Ok(());
                    }
                    let failed = record.kind == "error";
                    run.root.span().set_attribute(KeyValue::new(
                        "adk.status",
                        if failed { "error" } else { "done" },
                    ));
                    if failed {
                        run.root
                            .span()
                            .set_attribute(KeyValue::new("error.type", text("category")));
                        run.root.span().set_attribute(KeyValue::new(
                            "error.message",
                            data["error"].to_string(),
                        ));
                    }
                    runs.remove(&record.run_id)
                        .expect("active run")
                        .finish(failed);
                }
                _ => {}
            }
            Ok(())
        }
    }
    impl<T: Tracer + Send + Sync> EventSink for OtelBridge<T>
    where
        T::Span: Send + Sync + 'static,
    {
        fn emit<'a>(&'a self, record: &'a EventRecord) -> BoxFuture<'a, Result<(), Error>> {
            Box::pin(async move { self.record(record) })
        }
        fn finish_run<'a>(&'a self, run_id: &'a str) -> BoxFuture<'a, Result<(), Error>> {
            Box::pin(async move {
                if let Some(run) = self
                    .runs
                    .lock()
                    .map_err(|_| host_error("telemetry bridge poisoned"))?
                    .remove(run_id)
                {
                    run.finish(true);
                }
                Ok(())
            })
        }
        fn shutdown(&self) -> BoxFuture<'_, Result<(), Error>> {
            Box::pin(async move {
                let mut runs = self
                    .runs
                    .lock()
                    .map_err(|_| host_error("telemetry bridge poisoned"))?;
                for (_, run) in runs.drain() {
                    run.finish(true);
                }
                Ok(())
            })
        }
    }
}
