use adk_codec::{
    dto::{RawJson, ResponseSnapshot},
    snapshots::to_go_json,
};
use adk_core::*;
use serde_json::{Value, json};

fn response() -> ModelResponse {
    ModelResponse {
        items: vec![
            RunItem::PhasedMessage {
                message: Message {
                    role: Role::Assistant,
                    content: vec![
                        Content::Text {
                            text: "answer <>&".into(),
                        },
                        Content::Attachment {
                            media_type: "image/png".into(),
                            data: "AA==".into(),
                            detail: "low".into(),
                        },
                    ],
                },
                phase: "commentary".into(),
            },
            RunItem::Reasoning {
                reasoning: Reasoning {
                    id: "r".into(),
                    text: "reason".into(),
                    signature: "signature".into(),
                    redacted_data: "redacted".into(),
                    encrypted_content: "opaque".into(),
                },
            },
            RunItem::ToolCall {
                call: ToolCall {
                    id: "call".into(),
                    name: "lookup".into(),
                    arguments: json!({"q": 1}),
                },
            },
            RunItem::ToolResult {
                call_id: "call".into(),
                output: ToolOutput {
                    content: vec![Content::Text {
                        text: "result".into(),
                    }],
                    is_error: true,
                    should_pause: false,
                },
            },
            RunItem::Compaction {
                compaction: Compaction {
                    id: "c".into(),
                    content: "summary".into(),
                    encrypted_content: "compact".into(),
                    created_by: "provider".into(),
                },
            },
        ],
        usage: Usage {
            requests: 3,
            input_tokens: 12,
            output_tokens: 4,
            cache_read_tokens: 2,
            cache_creation_tokens: 1,
            context_tokens: Some(12),
        },
        end_turn: Some(false),
        response_id: Some("native-id".into()),
        metadata: Default::default(),
        snapshot_raw: None,
        raw: Some(json!({"answer": "<>&", "extra": [null, false, 1]})),
    }
}

#[test]
fn native_response_snapshot_bytes_match_independent_pinned_go() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    let variants = fixture["native_response_variants"].as_array().unwrap();
    let mut response = response();
    let raw = response.raw.clone();
    for (index, raw) in [raw, Some(Value::Null), None].into_iter().enumerate() {
        response.raw = raw;
        let snapshot = ResponseSnapshot::try_from(&response).unwrap();
        assert_eq!(
            to_go_json(&snapshot).unwrap(),
            variants[index].as_str().unwrap().as_bytes()
        );
        assert!(snapshot.items.iter().all(|item| item.agent_name.is_empty()));
    }
    response.items.clear();
    response.usage = Usage::default();
    response.end_turn = None;
    assert_eq!(
        to_go_json(&ResponseSnapshot::try_from(&response).unwrap()).unwrap(),
        variants[3].as_str().unwrap().as_bytes()
    );
}

#[test]
fn ordered_snapshot_raw_takes_precedence_after_native_roundtrips() {
    for text in ["{\"z\":1.00,\"a\":{\"y\":2,\"b\":3}}", "null"] {
        for raw in [Some(json!({"native": "distinct"})), Some(Value::Null), None] {
            let mut response = response();
            response.raw = raw;
            response.snapshot_raw = Some(JsonDocument::new(text.into()).unwrap());
            let encoded = serde_json::to_string(&response).unwrap();
            let value = serde_json::to_value(&response).unwrap();
            for response in [
                response,
                serde_json::from_str(&encoded).unwrap(),
                serde_json::from_value(value).unwrap(),
            ] {
                let snapshot = ResponseSnapshot::try_from(&response).unwrap();
                assert!(snapshot.raw_available);
                let RawJson::Encoded(raw) = &snapshot.raw else {
                    panic!("ordered snapshot raw must remain encoded");
                };
                assert_eq!(raw.get(), text);
                assert_eq!(serde_json::to_string(&snapshot.raw).unwrap(), text);
                assert_eq!(to_go_json(&snapshot.raw).unwrap(), text.as_bytes());
            }
        }
    }
}

#[test]
fn absent_snapshot_raw_preserves_native_raw_availability() {
    for raw in [None, Some(Value::Null), Some(json!({"native": "data"}))] {
        let mut response = response();
        response.raw = raw.clone();
        let snapshot = ResponseSnapshot::try_from(&response).unwrap();
        assert_eq!(snapshot.raw_available, raw.is_some());
        assert_eq!(snapshot.raw.is_missing(), raw.is_none());
        assert_eq!(snapshot.raw.value().as_deref(), raw.as_ref());
    }
}

#[test]
fn native_snapshot_rejects_unrepresentable_data_instead_of_dropping_it() {
    for field in [
        "requests",
        "input_tokens",
        "output_tokens",
        "cache_read_tokens",
        "cache_creation_tokens",
    ] {
        let mut value = serde_json::to_value(response()).unwrap();
        value["usage"][field] = json!(u64::MAX);
        let response: ModelResponse = serde_json::from_value(value).unwrap();
        assert!(ResponseSnapshot::try_from(&response).is_err(), "{field}");
    }
    for item in [
        RunItem::Message {
            message: Message {
                role: Role::Assistant,
                content: vec![Content::Image {
                    uri: "https://example.test/image".into(),
                    media_type: "image/png".into(),
                }],
            },
        },
        RunItem::Handoff {
            call_id: "call".into(),
            agent: "other".into(),
        },
        RunItem::ToolResult {
            call_id: "call".into(),
            output: ToolOutput {
                content: vec![],
                is_error: false,
                should_pause: true,
            },
        },
    ] {
        let mut response = response();
        response.items = vec![item];
        assert!(ResponseSnapshot::try_from(&response).is_err());
    }
}
