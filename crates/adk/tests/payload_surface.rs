#![cfg(feature = "compat")]

use adk::codec::{dto, from_go_json, to_go_json_bytes};

#[test]
fn payload_decoder_is_available_through_standalone_facade() {
    let message: dto::MessageOutput =
        from_go_json(br#"{"TEXT":"hello","text":null,"images":[null]}"#).unwrap();
    assert_eq!(message.text, "hello");
    assert_eq!(message.images, vec![dto::ImageAttachment::default()]);

    let call: dto::ToolCallData =
        from_go_json(br#"{"ID":"call","input":{"x":1e+02,"x":2}}"#).unwrap();
    assert_eq!(call.id, "call");
    assert_eq!(call.input.try_text().unwrap(), r#"{"x":1e+02,"x":2}"#);
    let unknown: dto::RunItemType = from_go_json(b"-42").unwrap();
    assert_eq!(unknown.0, -42);
    assert_eq!(to_go_json_bytes(&unknown).unwrap(), b"-42");
    let call: dto::ToolCallData = from_go_json(b"{\"input\":\"\xff\"}").unwrap();
    assert_eq!(call.input.bytes().as_ref(), b"\"\xff\"");
    assert!(call.input.try_text().is_err());
    assert_eq!(
        to_go_json_bytes(&call).unwrap(),
        b"{\"id\":\"\",\"name\":\"\",\"input\":\"\xff\"}"
    );
}

#[test]
fn item_helpers_are_available_through_standalone_facade() {
    use adk::codec::items::{extract_last_text, extract_text, extract_tool_calls};
    let items = [
        serde_json::from_slice(br#"{"Message":{"text":"hello"}}"#).unwrap(),
        serde_json::from_slice(br#"{"Type":1,"ToolCall":{"name":"bash","input":{"x":1e0}}}"#)
            .unwrap(),
        serde_json::from_slice(br#"{"Message":{"text":"world"}}"#).unwrap(),
    ];
    assert_eq!(extract_text(&items), "hello\nworld");
    assert_eq!(extract_last_text(&items), "world");
    let calls = extract_tool_calls(&items).unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "bash");
    assert_eq!(calls[0].input.bytes().as_ref(), br#"{"x":1e0}"#);
    assert!(extract_tool_calls(&[]).is_none());
}

#[cfg(feature = "observability")]
#[test]
fn logger_preserves_and_byte_truncates_invalid_raw_input() {
    use adk::agent_logger::{AgentLogger, LogLevel};
    use std::io::{Read, Seek};
    let mut file = tempfile::tempfile().unwrap();
    let logger = AgentLogger::new(LogLevel::DEBUG, file.try_clone().unwrap());
    let mut raw = vec![b'"'];
    raw.extend(std::iter::repeat_n(0xff, 310));
    raw.push(b'"');
    logger
        .input_items(&[dto::RunItem {
            kind: dto::RunItemType::TOOL_CALL,
            tool_call: Some(dto::ToolCallData {
                input: dto::RawJson::EncodedBytes(raw.clone()),
                ..Default::default()
            }),
            ..Default::default()
        }])
        .unwrap();
    logger.flush().unwrap();
    file.rewind().unwrap();
    let mut logged = Vec::new();
    file.read_to_end(&mut logged).unwrap();
    let mut expected = b" input=".to_vec();
    expected.extend_from_slice(&raw[..300]);
    expected.extend_from_slice("…\n".as_bytes());
    assert!(logged.ends_with(&expected));
}
