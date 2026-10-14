#![cfg(feature = "observability")]
use adk::{
    agent_logger::{AgentLogger, LogLevel, ModelResponseSummary, format_tool_names},
    codec::dto::*,
    content_stream::ContentStream,
};
use serde_json::Value;
use std::{
    fmt,
    io::{self, Write},
    sync::{Arc, Mutex},
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
fn raw(text: String) -> RawJson {
    RawJson::Encoded(serde_json::value::RawValue::from_string(text).unwrap())
}

#[test]
fn agent_logger_bodies_match_pinned_sdk() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/content-logger/observations.json"
    ))
    .unwrap();
    assert_eq!(
        fixture["agentLogLevels"],
        serde_json::json!([LogLevel::NORMAL.0, LogLevel::DEBUG.0])
    );
    assert_eq!(fixture["agentLogs"].as_array().unwrap().len(), 4);
    for observation in fixture["agentLogs"].as_array().unwrap() {
        let buffer = Buffer::default();
        let logger = AgentLogger::new(
            LogLevel(observation["level"].as_i64().unwrap()),
            buffer.clone(),
        );
        assert_eq!(logger.is_debug(), observation["debug"].as_bool().unwrap());
        logger.turn(-3, "agent\n😀", "model\"\\\0").unwrap();
        logger.turn_end(i64::MAX).unwrap();
        logger.tools(&[], &[], "").unwrap();
        logger
            .tools(
                &["shell".into(), "two words".into(), "😀\n".into()],
                &["handoff".into()],
                "safe\n\"",
            )
            .unwrap();
        logger.instructions("").unwrap();
        logger.instructions("instructions\n\x07").unwrap();
        logger.instructions(&"x".repeat(2000)).unwrap();
        for n in [1997, 1998, 1999, 2000, 2001] {
            logger
                .instructions(&format!("{}😀tail", "x".repeat(n)))
                .unwrap();
        }
        let mut items = vec![
            RunItem::default(),
            RunItem {
                agent: Some(AgentRef {
                    name: "worker".into(),
                }),
                message: Some(MessageOutput {
                    text: "answer\n".into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            RunItem {
                kind: RunItemType(1),
                tool_call: Some(ToolCallData {
                    id: "call".into(),
                    name: "shell".into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            RunItem {
                kind: RunItemType(1),
                tool_call: Some(ToolCallData {
                    id: "raw".into(),
                    name: "tool".into(),
                    input: raw("{ \"x\" : 1 }".into()),
                }),
                ..Default::default()
            },
            RunItem {
                kind: RunItemType(2),
                tool_output: Some(ToolOutputData {
                    call_id: "call".into(),
                    content: "result\n".into(),
                    is_error: true,
                    ..Default::default()
                }),
                ..Default::default()
            },
            RunItem {
                kind: RunItemType(7),
                ..Default::default()
            },
            RunItem {
                kind: RunItemType(7),
                compaction: Some(CompactionData {
                    id: "compact".into(),
                    encrypted_content: "secret😀".into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ];
        items.extend([
            RunItem {
                message: Some(MessageOutput {
                    text: "x".repeat(500),
                    ..Default::default()
                }),
                ..Default::default()
            },
            RunItem {
                kind: RunItemType(1),
                tool_call: Some(ToolCallData {
                    input: raw(format!("\"{}\"", "x".repeat(298))),
                    ..Default::default()
                }),
                ..Default::default()
            },
            RunItem {
                kind: RunItemType(2),
                tool_output: Some(ToolOutputData {
                    content: "x".repeat(300),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]);
        for kind in [3, 4, 5, 6, -1, i64::MAX] {
            items.push(RunItem {
                kind: RunItemType(kind),
                ..Default::default()
            });
        }
        for n in [497, 498, 499, 500, 501] {
            items.push(RunItem {
                message: Some(MessageOutput {
                    text: format!("{}😀tail", "x".repeat(n)),
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
        for n in [296, 297, 298, 299, 300, 301] {
            let text = format!("{}😀tail", "x".repeat(n));
            items.push(RunItem {
                kind: RunItemType(1),
                tool_call: Some(ToolCallData {
                    id: "long".into(),
                    name: "tool".into(),
                    input: raw(format!("\"{text}\"")),
                }),
                ..Default::default()
            });
            items.push(RunItem {
                kind: RunItemType(2),
                tool_output: Some(ToolOutputData {
                    call_id: "long".into(),
                    content: text,
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
        logger.input_items(&[]).unwrap();
        logger.input_items(&items).unwrap();
        logger.tool_exec("shell\n", "id", -7).unwrap();
        logger.tool_result("tool", "id", false, "").unwrap();
        logger.tool_result("tool", "id", true, "line\n").unwrap();
        logger
            .tool_result("tool", "id", false, &"x".repeat(1000))
            .unwrap();
        for n in [997, 998, 999, 1000, 1001] {
            logger
                .tool_result("tool", "long", false, &format!("{}😀tail", "x".repeat(n)))
                .unwrap();
        }
        logger
            .model_response(ModelResponseSummary {
                model: "model",
                text_count: -1,
                tool_count: 2,
                thinking_count: 3,
                input_tokens: i64::MIN,
                output_tokens: i64::MAX,
                stop_reason: "stop",
            })
            .unwrap();
        logger.parallel_tool_calls(&[]).unwrap();
        logger
            .parallel_tool_calls(&["tool".into(), "two words".into()])
            .unwrap();
        logger.auto_continue(-2, "reason\n").unwrap();
        logger
            .debug(format_args!("debug {} {}", "value", 7))
            .unwrap();
        logger
            .info(format_args!("info {} {}\n", "value", 7))
            .unwrap();
        logger.warn("warning\n").unwrap();
        logger
            .warn(format_args!("warning {} {}", "value", 7))
            .unwrap();
        logger.error("error\n").unwrap();
        logger
            .error(format_args!("error {} {}", "value", 7))
            .unwrap();
        logger
            .event(&ContentEvent {
                kind: "session_end".into(),
                status: "done".into(),
                ..Default::default()
            })
            .unwrap();
        let hex: String = buffer
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(
            hex,
            observation["logHex"].as_str().unwrap(),
            "level={}",
            logger.is_debug()
        );
    }
    assert_eq!(format_tool_names(&[]), "[]");
    assert_eq!(
        format_tool_names(&["one".into(), "two words".into()]),
        "[one two words]"
    );
}

#[derive(Default)]
struct State {
    writes: usize,
    flushes: usize,
    dropped: bool,
}
struct Failing(Arc<Mutex<State>>);
impl Write for Failing {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().writes += 1;
        Err(io::Error::other("unavailable"))
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap().flushes += 1;
        Err(io::Error::other("flush failed"))
    }
}
impl Drop for Failing {
    fn drop(&mut self) {
        self.0.lock().unwrap().dropped = true;
    }
}
struct MustNotFormat;
impl fmt::Display for MustNotFormat {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("disabled debug formatted")
    }
}

#[test]
fn multiline_logging_retains_first_error_across_successful_writes() {
    struct Intermittent(Arc<Mutex<usize>>);
    impl Write for Intermittent {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let mut calls = self.0.lock().unwrap();
            *calls += 1;
            match *calls {
                1 => Err(io::Error::new(io::ErrorKind::PermissionDenied, "first")),
                2 => Ok(bytes.len()),
                _ => Err(io::Error::new(io::ErrorKind::BrokenPipe, "last")),
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let calls = Arc::new(Mutex::new(0));
    let logger = AgentLogger::new(LogLevel::NORMAL, Intermittent(calls.clone()));
    let error = logger.tools(&[], &[], "").unwrap_err();
    assert_eq!(*calls.lock().unwrap(), 3);
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(error.to_string(), "first");
}

#[test]
fn agent_logger_keeps_writing_after_errors_and_shares_owned_event_sink() {
    let state = Arc::new(Mutex::new(State::default()));
    let logger = AgentLogger::new(LogLevel::NORMAL, Failing(state.clone()));
    logger.debug(MustNotFormat).unwrap();
    assert_eq!(state.lock().unwrap().writes, 0);
    assert!(logger.tools(&[], &[], "").is_err());
    assert_eq!(state.lock().unwrap().writes, 3);
    assert!(
        logger
            .input_items(&[RunItem::default(), RunItem::default()])
            .is_err()
    );
    assert_eq!(state.lock().unwrap().writes, 6);
    assert!(logger.flush().is_err());
    let stream = ContentStream::new(io::sink());
    stream.set_logger(Some(logger.event_logger()));
    drop(logger);
    assert!(!state.lock().unwrap().dropped);
    stream.emit(ContentEvent {
        kind: "status".into(),
        ..Default::default()
    });
    assert_eq!(state.lock().unwrap().writes, 7);
    drop(stream);
    assert!(state.lock().unwrap().dropped);
    assert_eq!(state.lock().unwrap().flushes, 1);

    let state = Arc::new(Mutex::new(State::default()));
    let logger = AgentLogger::new(LogLevel::DEBUG, Failing(state.clone()));
    assert!(logger.instructions(&"x".repeat(4001)).is_err());
    assert_eq!(state.lock().unwrap().writes, 4);
}
