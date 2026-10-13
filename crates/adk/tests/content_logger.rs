#![cfg(feature = "observability")]
use adk::{codec::dto::ContentEvent, content_logger::ContentLogger, content_stream::ContentStream};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{self, Write},
    sync::{Arc, Mutex, mpsc},
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
fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../fixtures/content-logger/observations.json"
    ))
    .unwrap()
}

#[test]
fn event_log_bodies_match_pinned_sdk() {
    let fixture = fixture();
    let buffer = Buffer::default();
    let logger = ContentLogger::new(buffer.clone());
    for case in fixture["cases"].as_array().unwrap() {
        buffer.0.lock().unwrap().clear();
        let event: ContentEvent = serde_json::from_value(case["event"].clone()).unwrap();
        logger.event(&event).unwrap();
        assert_eq!(
            *buffer.0.lock().unwrap(),
            case["log"].as_str().unwrap().as_bytes(),
            "{case}"
        );
    }
    buffer.0.lock().unwrap().clear();
    logger
        .event(&ContentEvent {
            kind: "phase_transition".into(),
            message: (0..=0x10ffff).filter_map(char::from_u32).collect(),
            ..Default::default()
        })
        .unwrap();
    let bytes = buffer.0.lock().unwrap();
    assert_eq!(
        bytes.len(),
        fixture["allUnicodeLogBytes"].as_u64().unwrap() as usize
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(&*bytes)),
        fixture["allUnicodeLogSHA256"].as_str().unwrap()
    );
}

#[test]
fn stream_logger_snapshots_and_annotations_match_sdk() {
    let buffer = Buffer::default();
    let logger = Arc::new(ContentLogger::new(buffer.clone()));
    let parent = ContentStream::new(io::sink());
    parent.set_logger(Some(logger.clone()));
    let child = parent.child("child");
    parent.set_logger(None);
    parent.emit(ContentEvent {
        kind: "not_logged".into(),
        ..Default::default()
    });
    child.emit(ContentEvent {
        kind: "subagent_status".into(),
        subagent_type: "researcher".into(),
        message: "first".into(),
        ..Default::default()
    });
    parent.child("later").emit(ContentEvent {
        kind: "not_logged".into(),
        ..Default::default()
    });
    child.set_logger(None);
    child.emit(ContentEvent {
        kind: "not_logged".into(),
        ..Default::default()
    });
    parent.set_logger(Some(logger));
    parent.emit(ContentEvent {
        kind: "subagent_status".into(),
        task_id: "child".into(),
        message: "inherited name".into(),
        ..Default::default()
    });
    parent.emit(ContentEvent {
        kind: "phase_transition".into(),
        message: "not serializable".into(),
        cost_usd: f64::NAN,
        ..Default::default()
    });
    assert_eq!(
        *buffer.0.lock().unwrap(),
        fixture()["streamLog"].as_str().unwrap().as_bytes()
    );
}

struct Probe {
    receiver: Arc<Mutex<Option<adk::content_stream::ContentSubscription>>>,
    log: bool,
    actions: Arc<Mutex<Vec<&'static str>>>,
}
impl Write for Probe {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        let guard = self.receiver.lock().unwrap();
        let received = guard.as_ref().unwrap().receiver.try_recv();
        if self.log {
            assert!(matches!(received, Err(mpsc::TryRecvError::Empty)));
            self.actions.lock().unwrap().push("log");
        } else {
            assert_eq!(received.unwrap().kind, "assistant_text");
            self.actions.lock().unwrap().push("jsonl");
        }
        Err(io::Error::other("destination unavailable"))
    }
    fn flush(&mut self) -> io::Result<()> {
        self.actions.lock().unwrap().push("flush");
        Ok(())
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        self.actions.lock().unwrap().push("drop");
    }
}

#[test]
fn log_precedes_delivery_and_jsonl_despite_writer_errors() {
    let actions = Arc::new(Mutex::new(Vec::new()));
    let receiver = Arc::new(Mutex::new(None));
    let parent = ContentStream::new(Probe {
        receiver: receiver.clone(),
        log: false,
        actions: actions.clone(),
    });
    let logger = Arc::new(ContentLogger::new(Probe {
        receiver: receiver.clone(),
        log: true,
        actions: actions.clone(),
    }));
    parent.set_logger(Some(logger.clone()));
    let child = parent.child("task");
    let subscription = parent.subscribe(4);
    *receiver.lock().unwrap() = Some(subscription);
    child.emit(ContentEvent {
        kind: "assistant_text".into(),
        ..Default::default()
    });
    assert_eq!(*actions.lock().unwrap(), ["log", "jsonl"]);
    logger.flush().unwrap();
    parent.set_logger(None);
    drop(logger);
    drop(parent);
    assert_eq!(*actions.lock().unwrap(), ["log", "jsonl", "flush"]);
    drop(child);
    assert_eq!(
        *actions.lock().unwrap(),
        ["log", "jsonl", "flush", "drop", "drop"]
    );
}
