use adk_codec::{
    dto::{RawJson, RunItem, RunItemType, ToolCallData},
    items::{extract_last_text, extract_text, extract_tool_calls},
};
use serde_json::{Value, value::RawValue};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../fixtures/item-helpers/observations.json"
    ))
    .unwrap()
}

#[test]
fn item_extractors_match_pinned_go_including_empty_messages_and_mismatched_payloads() {
    let fixture = fixture();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 136);
    for case in cases {
        let raw: Option<Vec<Box<RawValue>>> =
            serde_json::from_str(case["body"].as_str().unwrap()).unwrap();
        let items: Vec<RunItem> = raw
            .unwrap_or_default()
            .iter()
            .map(|item| serde_json::from_str(item.get()).unwrap())
            .collect();
        assert_eq!(
            extract_text(&items),
            case["text"].as_str().unwrap(),
            "{case}"
        );
        assert_eq!(
            extract_last_text(&items),
            case["last"].as_str().unwrap(),
            "{case}"
        );
        let calls = extract_tool_calls(&items);
        assert_eq!(
            calls.as_ref().map(Vec::len),
            case["calls"].as_array().map(Vec::len),
            "{case}"
        );
        if let Some(calls) = &calls {
            for (call, expected) in calls.iter().zip(case["calls"].as_array().unwrap()) {
                assert_eq!(call.id, expected["id"].as_str().unwrap());
                assert_eq!(call.name, expected["name"].as_str().unwrap());
            }
        }
        let raw: Vec<String> = calls
            .unwrap_or_default()
            .iter()
            .map(|call| {
                call.input
                    .bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect()
            })
            .collect();
        assert_eq!(
            serde_json::to_value(raw).unwrap(),
            case["call_inputs"],
            "{case}"
        );
    }
}

#[test]
fn tool_call_extraction_preserves_missing_empty_and_invalid_raw_input() {
    let fixture = fixture();
    let cases = fixture["raw_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    for case in cases {
        let text = case["input"].as_str().unwrap();
        let input = if case["nil"].as_bool().unwrap() {
            RawJson::Missing
        } else {
            RawJson::EncodedBytes(
                (0..text.len())
                    .step_by(2)
                    .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
                    .collect(),
            )
        };
        let items = [RunItem {
            kind: RunItemType::TOOL_CALL,
            tool_call: Some(ToolCallData {
                id: "raw".into(),
                name: "echo".into(),
                input,
            }),
            ..Default::default()
        }];
        let calls = extract_tool_calls(&items).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "raw");
        assert_eq!(calls[0].name, "echo");
        assert_eq!(
            calls[0].input.is_missing(),
            case["output_nil"].as_bool().unwrap()
        );
        let bytes: String = calls[0]
            .input
            .bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(bytes, case["output"].as_str().unwrap());
        assert_eq!(
            calls[0].input.bytes(),
            items[0].tool_call.as_ref().unwrap().input.bytes()
        );
    }
}
