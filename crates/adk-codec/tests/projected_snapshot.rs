use adk_codec::{
    dto::{RawJson, ResponseSnapshot, SnapshotType},
    snapshots::to_go_json,
};
use adk_core::{JsonDocument, ModelResponse, SnapshotProjection};
use serde_json::{Value, json};

fn response(raw: &str, projection: Option<SnapshotProjection>) -> ModelResponse {
    ModelResponse {
        items: vec![adk_core::RunItem::Handoff {
            call_id: "native".into(),
            agent: "other".into(),
        }],
        usage: adk_core::Usage {
            requests: u64::MAX,
            input_tokens: u64::MAX,
            ..Default::default()
        },
        end_turn: Some(true),
        response_id: Some("native".into()),
        metadata: Default::default(),
        raw: Some(json!({"native": true})),
        snapshot_raw: Some(JsonDocument::new(raw.into()).unwrap()),
        snapshot_projection: projection,
    }
}

fn normalized(content: Value) -> Value {
    json!({"id":"m", "type":"message", "role":"assistant", "content":content,
        "model":"fixture", "stop_reason":"", "usage":{"input_tokens":0,"output_tokens":0}})
}

fn roundtrips(response: ModelResponse) -> [ModelResponse; 3] {
    let encoded = serde_json::to_string(&response).unwrap();
    let value = serde_json::to_value(&response).unwrap();
    [
        response,
        serde_json::from_str(&encoded).unwrap(),
        serde_json::from_value(value).unwrap(),
    ]
}

#[test]
fn tagged_snapshots_byte_match_executed_public_sdk_fixtures() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    let mut checked = 0;
    for (name, case) in fixtures["provider_response_cases"].as_object().unwrap() {
        let projection = if case["protocol"] == "anthropic" {
            SnapshotProjection::GoAnthropicMessage
        } else {
            SnapshotProjection::GoOpenAiMessage
        };
        for (raw_key, snapshot_key) in [
            ("raw_json", "snapshot_json"),
            ("stream_raw_json", "stream_snapshot_json"),
        ] {
            let Some(raw) = case[raw_key].as_str() else {
                continue;
            };
            for response in roundtrips(response(raw, Some(projection))) {
                let before = response.clone();
                let snapshot = ResponseSnapshot::try_from(&response).unwrap();
                assert_eq!(
                    to_go_json(&snapshot).unwrap(),
                    case[snapshot_key].as_str().unwrap().as_bytes(),
                    "{name}: {snapshot_key}"
                );
                assert_eq!(response, before);
                let RawJson::Encoded(document) = snapshot.raw else {
                    panic!("encoded raw")
                };
                assert_eq!(document.get(), raw);
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 45);
}

#[test]
fn tagged_mapping_preserves_signed_usage_empty_items_and_raw_numbers() {
    let raw = r#"{"id":"m","type":"message","role":"assistant","content":[{"type":"text","phase":"commentary"},{"type":"text","text":"answer"},{"type":"tool_use","id":"t","name":"lookup","input":{"z":-0,"a":1.00,"e":1e+09,"huge":123456789012345678901234567890}},{"type":"thinking","id":"r","thinking":"reason","signature":"sig","encrypted_content":"enc","data":"ignored"},{"type":"redacted_thinking","id":"d","data":"redacted","encrypted_content":"opaque","thinking":"ignored","signature":"ignored"},{"type":"compaction","id":"c","content":"summary","encrypted_content":"compact","created_by":"gateway"},{"type":"image"}],"model":"fixture","stop_reason":"max_tokens","usage":{"input_tokens":-1,"output_tokens":-2,"cache_read_input_tokens":-3,"cache_creation_input_tokens":-4},"end_turn":false}"#;
    let input = r#"{"z":-0,"a":1.00,"e":1e+09,"huge":123456789012345678901234567890}"#;
    for projection in [
        SnapshotProjection::GoOpenAiMessage,
        SnapshotProjection::GoAnthropicMessage,
    ] {
        for response in roundtrips(response(raw, Some(projection))) {
            let snapshot = ResponseSnapshot::try_from(&response).unwrap();
            assert_eq!(snapshot.items.len(), 6);
            assert_eq!(snapshot.items[0].kind, SnapshotType::Message);
            assert_eq!(snapshot.items[0].message_text, "");
            assert_eq!(
                snapshot.items[0].message_phase,
                if projection == SnapshotProjection::GoOpenAiMessage {
                    "commentary"
                } else {
                    ""
                }
            );
            assert_eq!(
                snapshot.end_turn,
                if projection == SnapshotProjection::GoOpenAiMessage {
                    Some(false)
                } else {
                    None
                }
            );
            assert_eq!(snapshot.texts, ["answer"]);
            assert_eq!(snapshot.reasoning_texts, ["reason"]);
            assert_eq!(snapshot.thinking_texts, ["reason"]);
            assert_eq!(
                serde_json::to_value(&snapshot.reasoning).unwrap(),
                json!([
                    {"id":"r","text":"reason","thinking":"reason","signature":"sig","encrypted_content":"enc"},
                    {"id":"d","redacted_data":"redacted","encrypted_content":"opaque"}
                ])
            );
            assert_eq!(
                serde_json::to_value(&snapshot.items[5].compaction).unwrap(),
                json!({"id":"c","content":"summary","encrypted_content":"compact","created_by":"gateway"})
            );
            assert_eq!(
                serde_json::to_value(&snapshot.usage).unwrap(),
                json!({"requests":1,"input_tokens":-1,"output_tokens":-2,"cache_read_tokens":-3,"cache_create_tokens":-4})
            );
            assert_eq!(snapshot.tool_calls[0].id, "t");
            assert_eq!(snapshot.tool_calls[0].name, "lookup");
            for tool_input in [
                &snapshot.tool_calls[0].input,
                &snapshot.items[2].tool_call.as_ref().unwrap().input,
            ] {
                let RawJson::Encoded(encoded) = tool_input else {
                    panic!("encoded input")
                };
                assert_eq!(encoded.get(), input);
                assert_eq!(to_go_json(tool_input).unwrap(), input.as_bytes());
            }
            let RawJson::Encoded(document) = snapshot.raw else {
                panic!("encoded raw")
            };
            assert_eq!(document.get(), raw);
        }
    }
}

#[test]
fn tagged_documents_fail_closed_on_missing_or_malformed_shape() {
    for projection in [
        SnapshotProjection::GoOpenAiMessage,
        SnapshotProjection::GoAnthropicMessage,
    ] {
        let mut missing = response(&normalized(Value::Null).to_string(), Some(projection));
        missing.snapshot_raw = None;
        assert!(ResponseSnapshot::try_from(&missing).is_err());
        let valid = normalized(Value::Null);
        let mut invalid = vec![Value::Null, json!([]), json!({}), json!(false)];
        for key in [
            "id",
            "type",
            "role",
            "content",
            "model",
            "stop_reason",
            "usage",
        ] {
            let mut value = valid.clone();
            value.as_object_mut().unwrap().remove(key);
            invalid.push(value);
        }
        for (key, value) in [
            ("content", json!({})),
            ("content", json!([null])),
            ("content", json!([{}])),
            ("content", json!([{"type":"text","text":3}])),
            ("end_turn", json!(1)),
            ("usage", Value::Null),
            ("usage", json!({})),
        ] {
            let mut document = valid.clone();
            document[key] = value;
            invalid.push(document);
        }
        for key in [
            "input_tokens",
            "output_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ] {
            for bad in [Value::Null, json!("1"), json!(1.5), json!(u64::MAX)] {
                let mut value = valid.clone();
                value["usage"][key] = bad;
                invalid.push(value);
            }
        }
        for value in invalid {
            let response = response(&value.to_string(), Some(projection));
            assert!(ResponseSnapshot::try_from(&response).is_err(), "{value}");
        }
    }
}

#[test]
fn nullable_content_optional_end_turn_and_tool_input_follow_normalized_shape() {
    for projection in [
        SnapshotProjection::GoOpenAiMessage,
        SnapshotProjection::GoAnthropicMessage,
    ] {
        for end_turn in [
            None,
            Some(Value::Null),
            Some(json!(false)),
            Some(json!(true)),
        ] {
            let mut value = normalized(Value::Null);
            if let Some(end_turn) = end_turn {
                value["end_turn"] = end_turn;
            }
            let snapshot =
                ResponseSnapshot::try_from(&response(&value.to_string(), Some(projection)))
                    .unwrap();
            assert!(snapshot.items.is_empty());
            assert_eq!(snapshot.usage.requests, 1);
            assert_eq!(
                snapshot.end_turn,
                if projection == SnapshotProjection::GoOpenAiMessage {
                    value["end_turn"].as_bool()
                } else {
                    None
                }
            );
        }
        let value = normalized(json!([{"type":"tool_use"},{"type":"tool_use","input":null}]));
        let snapshot =
            ResponseSnapshot::try_from(&response(&value.to_string(), Some(projection))).unwrap();
        assert!(snapshot.tool_calls[0].input.is_missing());
        let RawJson::Encoded(input) = &snapshot.tool_calls[1].input else {
            panic!("encoded null")
        };
        assert_eq!(input.get(), "null");
    }
}

#[test]
fn untagged_normalized_shape_is_opaque_not_inferred() {
    let raw = normalized(json!([{"type":"text","text":"not native"}])).to_string();
    let mut response = response(&raw, None);
    response.items.clear();
    response.usage = Default::default();
    for response in roundtrips(response) {
        let snapshot = ResponseSnapshot::try_from(&response).unwrap();
        assert!(snapshot.items.is_empty());
        assert_eq!(snapshot.usage.requests, 0);
        assert_eq!(snapshot.end_turn, Some(true));
        let RawJson::Encoded(encoded) = snapshot.raw else {
            panic!("encoded raw")
        };
        assert_eq!(encoded.get(), raw);
    }
}
