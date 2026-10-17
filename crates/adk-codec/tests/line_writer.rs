use adk_codec::line_writer::{LineEvent, LineWriter};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::io::Write;

#[test]
fn line_writer_matches_pinned_sdk() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/line-writer/observations.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let events = RefCell::new(Vec::new());
        let mut writer = LineWriter::new(|event| {
            if case["nil_sink"].as_bool().unwrap() {
                return;
            }
            let row = match event {
                LineEvent::Log(text) => {
                    let mut row = json!({"kind":"log", "text_len":text.len(), "text_sha256":format!("{:x}", Sha256::digest(text.as_bytes()))});
                    if text.len() < 1024 {
                        row["text"] = text.into();
                    }
                    row
                }
                LineEvent::ChildTool(child) => json!({"kind":"child_tool", "child":child}),
                LineEvent::Subagent(subagent) => json!({"kind":"subagent", "subagent":subagent}),
            };
            events.borrow_mut().push(row);
        });
        for (chunk, expected) in case["chunks"]
            .as_array()
            .unwrap()
            .iter()
            .zip(case["writes"].as_array().unwrap())
        {
            let text: Vec<u8> = if let Some(text) = chunk.get("text") {
                text.as_str().unwrap().as_bytes().to_vec()
            } else {
                serde_json::from_value(chunk["bytes"].clone()).unwrap()
            };
            let mut bytes = text.repeat(chunk["repeat"].as_u64().unwrap() as usize);
            bytes.extend_from_slice(chunk["tail"].as_str().unwrap().as_bytes());
            bytes.splice(
                ..0,
                case["prefix"].as_str().unwrap().as_bytes().iter().copied(),
            );
            assert_eq!(
                writer.write(&bytes).unwrap(),
                expected["accepted"].as_u64().unwrap() as usize,
                "{name}"
            );
            assert_eq!(
                events.borrow().len(),
                expected["emitted_count"].as_u64().unwrap() as usize,
                "{name}"
            );
        }
        drop(writer);
        let mut expected = case["events"].as_array().unwrap().clone();
        for row in &mut expected {
            row.as_object_mut().unwrap().remove("raw_text_len");
        }
        assert_eq!(*events.borrow(), expected, "{name}");
    }
}

#[test]
fn explicit_flush_and_shared_host_ownership() {
    use std::sync::{Arc, Mutex};
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let writer = Arc::new(Mutex::new(LineWriter::new(move |event| {
        sink.lock().unwrap().push(event)
    })));
    let first = writer.clone();
    std::thread::spawn(move || first.lock().unwrap().write_all(b"first\n").unwrap())
        .join()
        .unwrap();
    writer.lock().unwrap().write_all(b"second").unwrap();
    assert_eq!(events.lock().unwrap().len(), 1);
    writer.lock().unwrap().flush().unwrap();
    writer.lock().unwrap().flush().unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            LineEvent::Log("first".into()),
            LineEvent::Log("second".into())
        ]
    );
    let mut discard = LineWriter::new(|_| {});
    discard.write_all(b"discard\npartial").unwrap();
    discard.flush().unwrap();
}

#[test]
fn original_line_writer_emits_child_tool_event() {
    let events = RefCell::new(Vec::new());
    let mut writer = LineWriter::new(|event| events.borrow_mut().push(event));
    writer.write_all(concat!(r#"{"type":"tool_end","agent_name":"researcher","tool":"Bash","tool_use_id":"child_1","parent_call_id":"parent_1","is_error":true,"output":"exit 2","tool_duration_ms":42}"#, "\n").as_bytes()).unwrap();
    let events = events.borrow();
    assert_eq!(events.len(), 1);
    let LineEvent::ChildTool(child) = &events[0] else {
        panic!("expected child tool event")
    };
    assert_eq!(child.parent_call_id, "parent_1");
    assert_eq!(child.duration_ms, 42);
}

#[test]
fn original_line_writer_emits_subagent_event() {
    let events = RefCell::new(Vec::new());
    let mut writer = LineWriter::new(|event| events.borrow_mut().push(event));
    writer.write_all(concat!(r#"{"type":"subagent_status","task_id":"task_1","agent_name":"researcher","status":"running","message":"running","subagent_waiting_on":["task_0"],"subagent_messages_received":2}"#, "\n").as_bytes()).unwrap();
    let events = events.borrow();
    assert_eq!(events.len(), 1);
    let LineEvent::Subagent(subagent) = &events[0] else {
        panic!("expected subagent event")
    };
    assert_eq!(subagent.task_id, "task_1");
    assert_eq!(subagent.agent_name, "researcher");
    assert_eq!(subagent.status, "running");
    assert_eq!(subagent.messages_received, 2);
    assert_eq!(subagent.waiting_on.as_ref().unwrap(), &["task_0"]);
}

#[test]
fn original_line_writer_buffers_partial_lines() {
    let events = RefCell::new(Vec::new());
    let mut writer = LineWriter::new(|event| events.borrow_mut().push(event));
    writer
        .write_all(br#"{"type":"tool_end","tool":"Bash","is_error":true,"output":"hello"#)
        .unwrap();
    assert!(events.borrow().is_empty());
    writer.write_all(b" world\"}\n").unwrap();
    let events = events.borrow();
    assert_eq!(events.len(), 1);
    let LineEvent::Log(text) = &events[0] else {
        panic!("expected compact log")
    };
    assert!(text.contains("hello world"));
}
