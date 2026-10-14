#![cfg(feature = "observability")]
use adk::{
    codec::{dto::ContentEvent, events::parse_content_event_line, snapshots::to_go_json},
    content_stream::{ContentStream, ContentSubscription},
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    sync::{Arc, Mutex, mpsc},
    time::SystemTime,
};

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn record(mut event: ContentEvent, start: DateTime<Utc>) -> Value {
    let timestamp = DateTime::parse_from_rfc3339(event.timestamp.as_str()).unwrap();
    let automatic = timestamp >= start;
    if automatic {
        let end: DateTime<Utc> = SystemTime::now().into();
        assert!(timestamp <= end);
        assert!(event.timestamp.as_str().ends_with('Z'));
        event.timestamp = Default::default();
    }
    let nonfinite = event.cost_usd.is_nan();
    if nonfinite {
        event.cost_usd = 0.0;
    }
    json!({"automatic":automatic,"nonfinite":nonfinite,"json":String::from_utf8(to_go_json(&event).unwrap()).unwrap()})
}

fn drain(sub: &ContentSubscription, start: DateTime<Utc>) -> Value {
    let mut events = Vec::new();
    let closed = loop {
        match sub.receiver.try_recv() {
            Ok(event) => events.push(record(event, start)),
            Err(mpsc::TryRecvError::Empty) => break false,
            Err(mpsc::TryRecvError::Disconnected) => break true,
        }
    };
    json!({"events":events,"closed":closed})
}

fn wire(buffer: &Buffer, start: DateTime<Utc>) -> Value {
    let bytes = buffer.0.lock().unwrap();
    let events: Vec<_> = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| record(serde_json::from_str(line).unwrap(), start))
        .collect();
    events.into()
}

#[test]
fn typed_helpers_match_pinned_sdk() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/content-stream/observations.json"
    ))
    .unwrap();
    let start: DateTime<Utc> = SystemTime::now().into();
    let output = Buffer::default();
    let root = ContentStream::new(output.clone());
    root.set_session(-7);
    root.set_phase("helpers");
    let child = root.child("task");
    let sub = child.subscribe(32);
    for text in [String::new(), "line\n\"<&>😀".into(), "😀".repeat(17000)] {
        child.emit_text(&text, "");
        child.emit_text(&text, "agent");
        child.emit_thinking(&text, "");
        child.emit_thinking(&text, "agent");
        child.emit_thinking_delta(&text, "agent", "attempt");
        child.emit_compaction(2147483648, -2147483649, &text);
        child.emit_hook_decision("hook", "tool", "deny", &text);
    }
    assert_eq!(wire(&output, start), fixture["helpers"]);
    assert_eq!(drain(&sub, start)["events"], fixture["helpers"]);
}

fn event(kind: &str) -> ContentEvent {
    ContentEvent {
        kind: kind.into(),
        ..Default::default()
    }
}

#[test]
fn content_stream_matches_pinned_sdk() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/content-stream/observations.json"
    ))
    .unwrap();
    let start: DateTime<Utc> = SystemTime::now().into();
    let output = Buffer::default();
    let root = ContentStream::new(output.clone());
    root.set_session(7);
    root.set_phase("planning");
    root.set_step("exploring");
    let child = root.child("task");
    root.set_session(8);
    root.set_phase("implementation");
    root.set_step("editing");
    let mut sub = child.subscribe(16);
    root.emit(ContentEvent {
        message: "message".into(),
        ..event("root")
    });
    child.emit(event("child"));
    child.emit(parse_content_event_line(r#"{"type":"explicit","session":9,"phase":"phase","step":"step","task_id":"override","ts":"2020-02-03T04:05:06.1234+05:30"}"#).unwrap());
    child.emit(
        parse_content_event_line(r#"{"type":"zero_offset","ts":"0001-01-01T01:00:00+01:00"}"#)
            .unwrap(),
    );
    child.emit(
        parse_content_event_line(
            r#"{"type":"zero_large_offset","ts":"0001-01-02T01:00:00+24:60"}"#,
        )
        .unwrap(),
    );
    root.set_session(0);
    root.set_phase("");
    root.set_step("");
    root.emit(event("cleared"));
    root.set_session(i32::MIN);
    root.set_phase("界\nphase");
    root.set_step(" \tstep");
    root.emit(event("minimum"));
    root.set_session(i32::MAX);
    root.emit(event("maximum"));
    assert_eq!(drain(&sub, start), fixture["metadata"]["subscriber"]);
    assert_eq!(wire(&output, start), fixture["metadata"]["wire"]);
    sub.close();
    sub.close();
    root.emit(event("after_close"));
    assert_eq!(drain(&sub, start), fixture["metadata"]["closed"]);

    let output = Buffer::default();
    let root = ContentStream::new(output.clone());
    let child = root.child("task");
    for event in [
        ContentEvent {
            task_id: "task".into(),
            subagent_type: "worker".into(),
            agent_name: "explicit".into(),
            ..event("subagent_status")
        },
        event("subagent_status"),
        ContentEvent {
            agent_name: "renamed".into(),
            ..event("subagent_status")
        },
        event("subagent_status"),
        event("other"),
        ContentEvent {
            task_id: "unknown".into(),
            ..event("subagent_status")
        },
    ] {
        child.emit(event);
    }
    assert_eq!(wire(&output, start), fixture["annotation"]);

    for case in fixture["drops"].as_array().unwrap() {
        let capacity = case["capacity"].as_i64().unwrap();
        let root = ContentStream::new(io::sink());
        let mut sub = root.subscribe(capacity.max(0) as usize);
        let count = if capacity <= 0 { 66 } else { 4 };
        for i in 0..count {
            root.emit(ContentEvent {
                message: i.to_string(),
                ..event("tick")
            });
        }
        assert_eq!(drain(&sub, start), case["first"], "{capacity}");
        root.emit(event("later"));
        assert_eq!(drain(&sub, start), case["second"], "{capacity}");
        root.emit(event("last"));
        assert_eq!(drain(&sub, start), case["third"], "{capacity}");
        sub.close();
        assert_eq!(drain(&sub, start), case["closed"], "{capacity}");
    }

    struct Failed(Arc<Mutex<usize>>);
    impl Write for Failed {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            *self.0.lock().unwrap() += 1;
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
    }
    let attempts = Arc::new(Mutex::new(0));
    let root = ContentStream::new(Failed(attempts.clone()));
    let sub = root.subscribe(4);
    root.emit(event("writer_failure"));
    root.emit(ContentEvent {
        cost_usd: f64::NAN,
        ..event("marshal_failure")
    });
    assert_eq!(
        *attempts.lock().unwrap(),
        fixture["failure"]["attempts"].as_u64().unwrap() as usize
    );
    assert_eq!(drain(&sub, start), fixture["failure"]["subscriber"]);
    assert_eq!(root.flush().unwrap_err().kind(), io::ErrorKind::BrokenPipe);
}

#[test]
fn subscriptions_and_last_stream_have_explicit_owned_lifetimes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Owned(Arc<AtomicUsize>);
    impl Write for Owned {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Drop for Owned {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let dropped = Arc::new(AtomicUsize::new(0));
    let root = ContentStream::new(Owned(dropped.clone()));
    let child = root.child("child");
    let mut first = root.subscribe(2);
    let second = child.subscribe(2);
    root.emit(ContentEvent {
        subagent_depends_on: vec!["original".into()],
        ..event("value")
    });
    first.receiver.recv().unwrap().subagent_depends_on[0] = "mutated".into();
    assert_eq!(
        second.receiver.recv().unwrap().subagent_depends_on,
        vec!["original"]
    );
    root.emit(event("queued"));
    first.close();
    assert_eq!(first.receiver.recv().unwrap().kind, "queued");
    assert!(first.receiver.recv().is_err());
    drop(root);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    drop(child);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(second.receiver.recv().unwrap().kind, "queued");
    assert!(second.receiver.recv().is_err());
}

#[test]
fn child_threads_preserve_complete_records_and_each_producers_order() {
    let output = Buffer::default();
    let root = ContentStream::new(output.clone());
    let sub = root.subscribe(256);
    let workers: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|task| {
            let child = root.child(task);
            std::thread::spawn(move || {
                for i in 0..100 {
                    child.emit(ContentEvent {
                        message: i.to_string(),
                        ..event("tick")
                    });
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let received: Vec<_> = sub.receiver.try_iter().collect();
    let bytes = output.0.lock().unwrap();
    let written: Vec<ContentEvent> = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for events in [&received, &written] {
        assert_eq!(events.len(), 200);
        for task in ["first", "second"] {
            let messages: Vec<_> = events
                .iter()
                .filter(|event| event.task_id == task)
                .map(|event| event.message.parse::<usize>().unwrap())
                .collect();
            assert_eq!(messages, (0..100).collect::<Vec<_>>());
        }
    }
}

#[test]
fn subscriber_drop_counts_do_not_affect_other_receivers() {
    let root = ContentStream::new(io::sink());
    let slow = root.subscribe(2);
    let fast = root.subscribe(2);
    for kind in ["a", "b", "c"] {
        root.emit(event(kind));
        assert_eq!(fast.receiver.recv().unwrap().kind, kind);
    }
    assert_eq!(slow.receiver.recv().unwrap().kind, "a");
    assert_eq!(slow.receiver.recv().unwrap().kind, "b");
    root.emit(event("d"));
    let marker = slow.receiver.recv().unwrap();
    assert_eq!(marker.kind, "events_dropped");
    assert_eq!(marker.message, "1 events dropped (slow subscriber)");
    assert_eq!(slow.receiver.recv().unwrap().kind, "d");
    assert_eq!(fast.receiver.recv().unwrap().kind, "d");
}
