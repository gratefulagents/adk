use adk_codec::{dto::ResponseSnapshot, snapshots::to_go_json};
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
