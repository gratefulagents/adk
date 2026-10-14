//! SDK content-event streams, distinct from native ordered observations.
mod runtime;
pub use runtime::ContentGenerationObserver;

mod llm;
pub use llm::LlmAttempt;

mod subagent;
pub use subagent::{SubagentCompleted, SubagentStarted, SubagentTaskStatus};

use crate::content_logger::ContentLogger;
use adk_codec::{dto::ContentEvent, snapshots::to_go_json, timestamp::GoTimestamp};
use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};
use std::{
    collections::BTreeMap,
    io::{self, Write},
    sync::{Arc, Mutex, Weak, mpsc},
    time::SystemTime,
};

#[derive(Clone, Default)]
struct StreamContext {
    session: i32,
    phase: String,
    step: String,
    task_id: String,
    logger: Option<Arc<ContentLogger>>,
}

struct Subscriber {
    sender: mpsc::SyncSender<ContentEvent>,
    dropped: i64,
}

#[derive(Default)]
struct Bus {
    next_id: u64,
    subscribers: BTreeMap<u64, Subscriber>,
    subagent_names: BTreeMap<String, String>,
}

struct Shared {
    writer: Mutex<Box<dyn Write + Send>>,
    bus: Mutex<Bus>,
}

/// Owned SDK JSONL writer and subscription bus with copied child context.
///
/// Automatic timestamps use UTC. Explicit timestamp offsets are retained. This
/// is not the native observation bus or a runtime-session builder.
/// Writes are best effort, after subscription delivery, as in the SDK. Dropping
/// the last stream releases the writer and closes remaining subscription senders.
///
/// ```
/// use adk::{codec::dto::ContentEvent, content_stream::ContentStream};
/// let stream = ContentStream::new(std::io::sink());
/// stream.set_session(7);
/// let mut subscription = stream.subscribe(4);
/// stream.child("task").emit(ContentEvent {
///     kind: "status".into(), ..Default::default()
/// });
/// let event = subscription.receiver.recv().unwrap();
/// assert_eq!((event.session, event.task_id.as_str()), (7, "task"));
/// subscription.close();
/// stream.flush().unwrap();
/// ```
pub struct ContentStream {
    context: Mutex<StreamContext>,
    shared: Arc<Shared>,
}

/// Host-owned session metadata and optional initialization event.
///
/// Supply `AgentLogger::event_logger()` to enable logging at a host-selected
/// level and destination. No process-global logger is installed.
#[derive(Default)]
pub struct SessionStreamOptions {
    pub session: i32,
    pub phase: String,
    pub logger: Option<Arc<ContentLogger>>,
    pub system_init: Option<SystemInit>,
}

#[derive(Clone, Debug, Default)]
pub struct SystemInit {
    pub model: String,
    pub permission_mode: String,
    pub cwd: String,
    pub max_turns: i64,
    pub tools: Vec<String>,
    pub mcp_servers: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SessionEnd {
    pub status: String,
    pub cost_usd: f64,
    pub cost_known: bool,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read: i64,
    pub cache_create: i64,
    pub num_turns: i64,
    pub duration_ms: i64,
    pub stop_reason: String,
}

/// SDK tool-event input/output cap, including the ASCII truncation marker.
pub const DEFAULT_MAX_TOOL_RESULT_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, Default)]
pub struct ToolStart {
    pub tool: String,
    pub call_id: String,
    pub parent_call_id: String,
    pub summary: String,
    pub agent_name: String,
    pub input_raw: String,
}

#[derive(Clone, Debug, Default)]
pub struct ToolEnd {
    pub tool: String,
    pub call_id: String,
    pub parent_call_id: String,
    pub is_error: bool,
    pub agent_name: String,
    pub output: String,
    pub duration_ms: i64,
}

impl ContentStream {
    /// Construct a session and emit optional initialization before returning.
    /// Subscribe after construction to receive subsequent events, not init replay.
    ///
    /// ```
    /// use adk::{agent_logger::{AgentLogger, LogLevel},
    ///     content_stream::{ContentStream, SessionStreamOptions, SystemInit, SessionEnd}};
    /// let logger = AgentLogger::new(LogLevel::NORMAL, std::io::sink());
    /// let stream = ContentStream::session(std::io::sink(), SessionStreamOptions {
    ///     session: 2,
    ///     phase: "plan".into(),
    ///     logger: Some(logger.event_logger()),
    ///     system_init: Some(SystemInit { model: "offline".into(), ..Default::default() }),
    /// });
    /// stream.emit_session_end(SessionEnd { status: "completed".into(), ..Default::default() });
    /// stream.flush().unwrap();
    /// logger.flush().unwrap();
    /// ```
    pub fn session(writer: impl Write + Send + 'static, options: SessionStreamOptions) -> Self {
        let stream = Self::new(writer);
        stream.set_session(options.session);
        stream.set_phase(options.phase);
        stream.set_logger(options.logger);
        if let Some(init) = options.system_init {
            stream.emit_system_init(init);
        }
        stream
    }

    pub fn new(writer: impl Write + Send + 'static) -> Self {
        Self {
            context: Mutex::new(StreamContext::default()),
            shared: Arc::new(Shared {
                writer: Mutex::new(Box::new(writer)),
                bus: Mutex::new(Bus::default()),
            }),
        }
    }

    /// Share the writer and bus, but snapshot the parent's current metadata.
    pub fn child(&self, task_id: impl Into<String>) -> Self {
        let mut context = self
            .context
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        context.task_id = task_id.into();
        Self {
            context: Mutex::new(context),
            shared: self.shared.clone(),
        }
    }

    pub fn set_session(&self, session: i32) {
        self.context
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .session = session;
    }

    /// Attach or detach an owned logger. Existing children retain their snapshot.
    pub fn set_logger(&self, logger: Option<Arc<ContentLogger>>) {
        self.context
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .logger = logger;
    }

    pub fn set_phase(&self, phase: impl Into<String>) {
        self.context.lock().unwrap_or_else(|p| p.into_inner()).phase = phase.into();
    }

    pub fn set_step(&self, step: impl Into<String>) {
        self.context.lock().unwrap_or_else(|p| p.into_inner()).step = step.into();
    }

    /// Subscribe without blocking emission. Zero capacity selects the SDK default of 64.
    pub fn subscribe(&self, capacity: usize) -> ContentSubscription {
        let (sender, receiver) = mpsc::sync_channel(if capacity == 0 { 64 } else { capacity });
        let mut bus = self.shared.bus.lock().unwrap_or_else(|p| p.into_inner());
        let id = bus.next_id;
        bus.next_id += 1;
        bus.subscribers
            .insert(id, Subscriber { sender, dropped: 0 });
        ContentSubscription {
            receiver,
            shared: Arc::downgrade(&self.shared),
            id: Some(id),
        }
    }

    pub fn emit(&self, mut event: ContentEvent) {
        let logger = {
            let context = self.context.lock().unwrap_or_else(|p| p.into_inner());
            if event.session == 0 {
                event.session = context.session;
            }
            if event.phase.is_empty() {
                event.phase.clone_from(&context.phase);
            }
            if event.step.is_empty() {
                event.step.clone_from(&context.step);
            }
            if event.task_id.is_empty() {
                event.task_id.clone_from(&context.task_id);
            }
            context.logger.clone()
        };
        if zero_timestamp(&event.timestamp) {
            event.timestamp = now();
        }
        {
            let mut bus = self.shared.bus.lock().unwrap_or_else(|p| p.into_inner());
            if event.kind == "subagent_status" && !event.task_id.is_empty() {
                let name = if event.subagent_type.is_empty() {
                    &event.agent_name
                } else {
                    &event.subagent_type
                };
                if !name.is_empty() {
                    bus.subagent_names
                        .insert(event.task_id.clone(), name.clone());
                } else if let Some(name) = bus.subagent_names.get(&event.task_id) {
                    event.subagent_type.clone_from(name);
                    if event.agent_name.is_empty() {
                        event.agent_name.clone_from(name);
                    }
                }
            }
        }
        if let Some(logger) = logger {
            let _ = logger.event(&event);
        }
        {
            let mut bus = self.shared.bus.lock().unwrap_or_else(|p| p.into_inner());
            for subscriber in bus.subscribers.values_mut() {
                if subscriber.dropped > 0 {
                    let marker = ContentEvent {
                        timestamp: now(),
                        kind: "events_dropped".into(),
                        message: format!("{} events dropped (slow subscriber)", subscriber.dropped),
                        ..Default::default()
                    };
                    if subscriber.sender.try_send(marker).is_err() {
                        subscriber.dropped = subscriber.dropped.wrapping_add(1);
                        continue;
                    }
                    subscriber.dropped = 0;
                }
                if subscriber.sender.try_send(event.clone()).is_err() {
                    subscriber.dropped = subscriber.dropped.wrapping_add(1);
                }
            }
        }
        if let Ok(mut bytes) = to_go_json(&event) {
            bytes.push(b'\n');
            let _ = self
                .shared
                .writer
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .write(&bytes);
        }
    }

    /// Emit a tool start, capping only raw input at the SDK's UTF-8-safe byte limit.
    pub fn emit_tool_start(&self, start: ToolStart) {
        self.emit(ContentEvent {
            kind: "tool_start".into(),
            tool: start.tool,
            tool_use_id: start.call_id,
            parent_call_id: start.parent_call_id,
            message: start.summary,
            agent_name: start.agent_name,
            input_raw: truncate_tool_content(start.input_raw),
            ..Default::default()
        });
    }

    /// Both successful and failed tool outputs use the content-event cap.
    /// Persist uncapped outputs separately through the trace-store API.
    pub fn emit_tool_end(&self, end: ToolEnd) {
        self.emit(ContentEvent {
            kind: "tool_end".into(),
            tool: end.tool,
            tool_use_id: end.call_id,
            parent_call_id: end.parent_call_id,
            is_error: end.is_error,
            agent_name: end.agent_name,
            output: truncate_tool_content(end.output),
            tool_duration_ms: end.duration_ms,
            ..Default::default()
        });
    }

    pub fn emit_system_init(&self, init: SystemInit) {
        self.emit(ContentEvent {
            kind: "system_init".into(),
            model: init.model,
            permission_mode: init.permission_mode,
            cwd: init.cwd,
            max_turns: init.max_turns as i32,
            tools: init.tools,
            mcp_servers: init.mcp_servers,
            ..Default::default()
        });
    }

    pub fn emit_session_end(&self, end: SessionEnd) {
        self.emit(ContentEvent {
            kind: "session_end".into(),
            status: end.status,
            cost_usd: end.cost_usd,
            cost_known: end.cost_known,
            input_tokens: end.input_tokens,
            output_tokens: end.output_tokens,
            cache_read_input_tokens: end.cache_read,
            cache_creation_input_tokens: end.cache_create,
            num_turns: end.num_turns as i32,
            duration_ms: end.duration_ms,
            stop_reason: end.stop_reason,
            ..Default::default()
        });
    }

    pub fn emit_text(&self, text: impl Into<String>, agent_name: impl Into<String>) {
        self.emit(ContentEvent {
            kind: "assistant_text".into(),
            message: text.into(),
            agent_name: agent_name.into(),
            ..Default::default()
        });
    }

    pub fn emit_thinking(&self, thinking: impl Into<String>, agent_name: impl Into<String>) {
        self.emit(ContentEvent {
            kind: "assistant_thinking".into(),
            message: thinking.into(),
            agent_name: agent_name.into(),
            ..Default::default()
        });
    }

    pub fn emit_thinking_delta(
        &self,
        chunk: impl Into<String>,
        agent_name: impl Into<String>,
        attempt_id: impl Into<String>,
    ) {
        self.emit(ContentEvent {
            kind: "assistant_thinking_delta".into(),
            message: chunk.into(),
            agent_name: agent_name.into(),
            tool_use_id: attempt_id.into(),
            ..Default::default()
        });
    }

    /// Counts narrow to signed 32 bits, matching the SDK's 64-bit host conversion.
    pub fn emit_compaction(
        &self,
        tokens_before: i64,
        tokens_after: i64,
        summary: impl Into<String>,
    ) {
        self.emit(ContentEvent {
            kind: "compact_boundary".into(),
            message: "Context compacted".into(),
            output: summary.into(),
            tokens_before: tokens_before as i32,
            tokens_after: tokens_after as i32,
            ..Default::default()
        });
    }

    pub fn emit_hook_decision(
        &self,
        hook_name: impl Into<String>,
        tool_name: impl Into<String>,
        decision: impl Into<String>,
        reason: impl Into<String>,
    ) {
        self.emit(ContentEvent {
            kind: "hook_decision".into(),
            hook_name: hook_name.into(),
            tool: tool_name.into(),
            decision: decision.into(),
            reason: reason.into(),
            ..Default::default()
        });
    }

    /// Explicit host flush; emitting and dropping streams do not flush the writer.
    pub fn flush(&self) -> io::Result<()> {
        self.shared
            .writer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .flush()
    }
}

/// A bounded receiver. Close or drop unsubscribes; closing retains queued events.
/// Each subscriber receives independently owned event data rather than aliased Go slices.
pub struct ContentSubscription {
    pub receiver: mpsc::Receiver<ContentEvent>,
    shared: Weak<Shared>,
    id: Option<u64>,
}

impl ContentSubscription {
    pub fn close(&mut self) {
        if let Some(id) = self.id.take()
            && let Some(shared) = self.shared.upgrade()
        {
            shared
                .bus
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .subscribers
                .remove(&id);
        }
    }
}

impl Drop for ContentSubscription {
    fn drop(&mut self) {
        self.close();
    }
}

fn truncate_tool_content(mut text: String) -> String {
    if text.len() > DEFAULT_MAX_TOOL_RESULT_BYTES {
        let mut end = DEFAULT_MAX_TOOL_RESULT_BYTES - 3;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("...");
    }
    text
}

fn now() -> GoTimestamp {
    let timestamp: DateTime<Utc> = SystemTime::now().into();
    serde_json::from_value(timestamp.to_rfc3339_opts(SecondsFormat::Nanos, true).into())
        .expect("system time in supported RFC3339 range")
}

fn zero_timestamp(timestamp: &GoTimestamp) -> bool {
    let text = timestamp.as_str();
    let (date, offset) = if let Some(date) = text.strip_suffix('Z') {
        (date, 0)
    } else {
        let (date, zone) = text.split_at(text.len() - 6);
        let minutes: i64 = zone[4..].parse().expect("validated timestamp minute");
        let hours: i64 = zone[1..3].parse().expect("validated timestamp hour");
        let seconds = hours * 3600 + minutes * 60;
        (
            date,
            if zone.starts_with('-') {
                -seconds
            } else {
                seconds
            },
        )
    };
    let timestamp = NaiveDateTime::parse_from_str(date, "%Y-%m-%dT%H:%M:%S%.f")
        .expect("validated timestamp date")
        .and_utc();
    timestamp.timestamp() - offset == -62_135_596_800 && timestamp.timestamp_subsec_nanos() == 0
}
