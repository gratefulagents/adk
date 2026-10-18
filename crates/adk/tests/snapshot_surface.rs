#![cfg(feature = "compat")]

use adk::codec::{
    dto::{
        CompactionSnapshot, ReasoningSnapshot, ResponseSnapshot, RunItemSnapshot,
        ToolApprovalSnapshot, ToolCallSnapshot,
    },
    from_go_json,
    snapshots::RequestSnapshot,
    to_go_json_bytes,
};

#[test]
fn public_snapshot_slice_state_matches_pinned_go() {
    use serde_json::{Value, json};
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    for case in fixture["request_record_cases"].as_array().unwrap() {
        if case["accepted"] != true {
            continue;
        }
        let snapshot: RequestSnapshot =
            from_go_json(case["input"].as_str().unwrap().as_bytes()).unwrap();
        assert_eq!(
            json!({
                "input_items": snapshot.input_items.is_none(),
                "tools": snapshot.tools.is_none(),
                "stop_sequences": snapshot.settings.stop_sequences.is_none(),
                "message_images": snapshot.input_items.iter().flatten().map(|item| item.message_images.is_none()).collect::<Vec<_>>()
            }),
            case["nil_slices"]
        );
    }
    for case in fixture["response_record_cases"].as_array().unwrap() {
        if case["accepted"] != true {
            continue;
        }
        let snapshot: ResponseSnapshot =
            from_go_json(case["input"].as_str().unwrap().as_bytes()).unwrap();
        assert_eq!(
            json!({
                "items": snapshot.items.is_none(), "texts": snapshot.texts.is_none(),
                "reasoning": snapshot.reasoning.is_none(), "reasoning_texts": snapshot.reasoning_texts.is_none(),
                "thinking_texts": snapshot.thinking_texts.is_none(), "tool_calls": snapshot.tool_calls.is_none(),
                "message_images": snapshot.items.iter().flatten().map(|item| item.message_images.is_none()).collect::<Vec<_>>()
            }),
            case["nil_slices"]
        );
    }
}

fn unhex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn public_request_response_snapshot_bytes_match_pinned_go() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/payloads/observations.json")).unwrap();
    let cases = fixture["envelope_byte_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 130);
    for (index, case) in cases.iter().enumerate() {
        let input = unhex(case["input_hex"].as_str().unwrap());
        let encoded = match case["type"].as_str().unwrap() {
            "request" => {
                from_go_json::<RequestSnapshot>(&input).and_then(|item| to_go_json_bytes(&item))
            }
            "response" => {
                from_go_json::<ResponseSnapshot>(&input).and_then(|item| to_go_json_bytes(&item))
            }
            other => panic!("unexpected envelope {other}"),
        };
        assert_eq!(
            encoded.is_ok(),
            case["decode_ok"].as_bool().unwrap(),
            "case {index}"
        );
        if let Ok(bytes) = encoded {
            assert_eq!(case["marshal_ok"], true, "case {index}");
            assert!(
                bytes == unhex(case["marshal_hex"].as_str().unwrap()),
                "encoded case {index}"
            );
        }
    }
}

#[test]
fn public_run_item_snapshot_bytes_match_pinned_go() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/payloads/observations.json")).unwrap();
    let cases = fixture["run_item_byte_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 421);
    for (index, case) in cases.iter().enumerate() {
        let decoded = from_go_json::<RunItemSnapshot>(&unhex(case["input_hex"].as_str().unwrap()));
        assert_eq!(
            decoded.is_ok(),
            case["decode_ok"].as_bool().unwrap(),
            "case {index}"
        );
        if let Ok(item) = decoded {
            assert_eq!(
                item.message_images.is_none(),
                case["message_images_nil"],
                "case {index}"
            );
            assert_eq!(
                serde_json::to_value(
                    item.tool_output
                        .as_ref()
                        .map(|output| output.images.is_none())
                )
                .unwrap(),
                case["output_images_nil"],
                "case {index}"
            );
            let encoded = to_go_json_bytes(&item);
            assert_eq!(
                encoded.is_ok(),
                case["marshal_ok"].as_bool().unwrap(),
                "case {index}"
            );
            if let Ok(bytes) = encoded {
                assert_eq!(
                    bytes,
                    unhex(case["marshal_hex"].as_str().unwrap()),
                    "case {index}"
                );
            }
        }
    }
}

#[test]
fn public_leaf_snapshot_byte_codecs_match_pinned_go() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/payloads/observations.json")).unwrap();
    let mut seen = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let input = unhex(case["input_hex"].as_str().unwrap());
        let result =
            match case["type"].as_str().unwrap() {
                "LLMReasoning" => from_go_json::<ReasoningSnapshot>(&input)
                    .and_then(|value| to_go_json_bytes(&value)),
                "LLMCompaction" => from_go_json::<CompactionSnapshot>(&input)
                    .and_then(|value| to_go_json_bytes(&value)),
                "LLMToolCall" => from_go_json::<ToolCallSnapshot>(&input)
                    .and_then(|value| to_go_json_bytes(&value)),
                "LLMToolApproval" => from_go_json::<ToolApprovalSnapshot>(&input)
                    .and_then(|value| to_go_json_bytes(&value)),
                _ => continue,
            };
        seen += 1;
        assert_eq!(
            result.is_ok(),
            case["decode_ok"].as_bool().unwrap(),
            "{}",
            case["id"]
        );
        if let Ok(encoded) = result {
            assert_eq!(case["marshal_ok"], true, "{}", case["id"]);
            assert_eq!(
                encoded,
                unhex(case["marshal_hex"].as_str().unwrap()),
                "{}",
                case["id"]
            );
        }
    }
    assert_eq!(seen, 693);
}
