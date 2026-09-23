use adk_core::ModelEvent;
use adk_providers::{
    client::StreamState,
    sse::Decoder,
    wire::{self, Protocol},
};
use serde_json::Value;

fn cases() -> Value {
    serde_json::from_str::<Value>(include_str!("../../../fixtures/tracestore/sdk-writer.json"))
        .unwrap()["provider_response_cases"]
        .clone()
}

#[test]
fn chat_normalized_raw_matches_independently_executed_public_sdk() {
    let cases = cases();
    let mut count = 0;
    for (name, case) in cases.as_object().unwrap() {
        if case["protocol"] != "chat" {
            continue;
        }
        let body = case["body"].as_str().unwrap();
        let response = wire::response_json(body.as_bytes(), Protocol::Chat).unwrap();
        assert_eq!(response.raw, Some(serde_json::from_str(body).unwrap()));
        assert_eq!(
            response.snapshot_raw.unwrap().as_str(),
            case["raw_json"].as_str().unwrap(),
            "{name}"
        );
        count += 1;
    }
    assert_eq!(count, 2);
}

#[test]
fn streamed_normalized_raw_matches_independently_executed_public_sdk() {
    let cases = cases();
    let mut count = 0;
    for (name, case) in cases.as_object().unwrap() {
        let expected = case["stream_raw_json"].as_str().or_else(|| {
            (name == "anthropic_stream_text_tool").then(|| case["raw_json"].as_str().unwrap())
        });
        let Some(expected) = expected else {
            continue;
        };
        let protocol = match case["protocol"].as_str().unwrap() {
            "responses" => Protocol::Responses,
            "anthropic" => Protocol::Anthropic,
            _ => unreachable!(),
        };
        let body = case["body"].as_str().unwrap().as_bytes();
        let mut decoder = Decoder::default();
        let mut stream = StreamState::new(protocol);
        let mut complete = None;
        for event in decoder.feed(body).unwrap() {
            for event in stream
                .event(&event.data)
                .unwrap_or_else(|e| panic!("{name}: {e:?}"))
            {
                if let ModelEvent::Complete { response } = event {
                    complete = Some(response);
                }
            }
        }
        decoder.finish().unwrap();
        let response = complete.unwrap_or_else(|| panic!("{name}: no completion"));
        assert!(response.raw.is_some());
        assert_eq!(response.snapshot_raw.unwrap().as_str(), expected, "{name}");
        count += 1;
    }
    assert_eq!(count, 7);
}

#[test]
fn ordered_direct_document_retains_raw_fragments_and_null_defaults() {
    let body = br#"{"id":null,"role":null,"type":null,"model":null,"stop_reason":null,"usage":null,"content":[{"type":"tool_use","id":"c","name":"tool","input":{ "z":1e+02, "a":2.00, "z":3 }}]}"#;
    let response = wire::response_json(body, Protocol::Anthropic).unwrap();
    let document = response.snapshot_raw.unwrap();
    assert!(
        document
            .as_str()
            .contains(r#""input":{"z":1e+02,"a":2.00,"z":3}"#)
    );
    assert!(
        document
            .as_str()
            .starts_with(r#"{"id":"","type":"","role":"""#)
    );
    assert!(
        document
            .as_str()
            .ends_with(r#""usage":{"input_tokens":0,"output_tokens":0}}"#)
    );
}

#[test]
fn permissive_sdk_identity_defaults_do_not_allow_invalid_stream_order() {
    let mut stream = StreamState::new(Protocol::Anthropic);
    assert!(stream.event(r#"{"type":"message_stop"}"#).is_err());
    assert!(stream.event(r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#).is_err());
    stream
        .event(r#"{"type":"message_start","message":{}}"#)
        .unwrap();
    assert!(
        stream
            .event(r#"{"type":"message_start","message":{}}"#)
            .is_err()
    );
    let mut stream = StreamState::new(Protocol::Responses);
    assert!(stream.event(r#"{"type":"response.function_call_arguments.done","arguments":"{}","output_index":3}"#).is_err());
}

#[test]
fn anthropic_terminal_defaults_do_not_accept_unfinished_blocks() {
    for stop_reason in [false, true] {
        let mut stream = StreamState::new(Protocol::Anthropic);
        stream
            .event(r#"{"type":"message_start","message":{"id":"m","content":[]}}"#)
            .unwrap();
        stream.event(r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#).unwrap();
        stream.event(r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"partial"}}"#).unwrap();
        if stop_reason {
            stream
                .event(r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#)
                .unwrap();
        }
        let error = stream.event(r#"{"type":"message_stop"}"#).unwrap_err();
        assert_eq!(error.info.message, "unfinished content block");
    }
}

#[test]
fn snapshot_capture_does_not_fail_tolerated_invalid_model_arguments() {
    for input in [r#"{"x":"\uD800"}"#, r#"{"x":"\uDC00"}"#, "{", ""] {
        let body = serde_json::json!({"choices":[{"message":{"tool_calls":[{"id":"c","function":{"name":"f","arguments":input}}]}}]});
        let response = wire::response(&body, Protocol::Chat).unwrap();
        assert!(
            matches!(&response.items[0], adk_core::RunItem::ToolCall { call } if call.arguments == serde_json::json!({}))
        );
        assert!(
            response
                .snapshot_raw
                .unwrap()
                .as_str()
                .contains(r#""input":{}"#)
        );
    }
}

#[test]
fn native_decoded_argument_objects_remain_visible_in_snapshots() {
    let body = serde_json::json!({"choices":[{"message":{"tool_calls":[{"id":"c","function":{"name":"f","arguments":{"x":1}}}]}}]});
    let response = wire::response(&body, Protocol::Chat).unwrap();
    let raw: Value = serde_json::from_str(response.snapshot_raw.unwrap().as_str()).unwrap();
    assert_eq!(raw["content"][0]["input"], serde_json::json!({"x":1}));
}
