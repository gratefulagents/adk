//! SDK content-event streams, distinct from native ordered observations.
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
/// is not the native observation bus, a stdout logger or a session builder.
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

impl ContentStream {
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
