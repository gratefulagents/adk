use adk_codec::{
    GoJsonDecode, GoJsonEncode,
    dto::*,
    from_go_json,
    snapshots::{OutputSchema, RequestSnapshot, ToolSnapshot},
    to_go_json_bytes,
};
use serde_json::Value;
use std::collections::BTreeMap;

fn unhex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/payloads/observations.json")).unwrap()
}
fn item_slice_states(prefix: &str, items: &[RunItemSnapshot], states: &mut BTreeMap<String, bool>) {
    for (index, item) in items.iter().enumerate() {
        states.insert(
            format!("{prefix}/{index}/message_images"),
            item.message_images.is_none(),
        );
        if let Some(output) = &item.tool_output {
            states.insert(
                format!("{prefix}/{index}/tool_output/images"),
                output.images.is_none(),
            );
        }
    }
}
fn request_slice_states(item: &RequestSnapshot) -> BTreeMap<String, bool> {
    let mut states = BTreeMap::from([
        ("input_items".into(), item.input_items.is_none()),
        ("tools".into(), item.tools.is_none()),
        (
            "settings/stop_sequences".into(),
            item.settings.stop_sequences.is_none(),
        ),
    ]);
    item_slice_states(
        "input_items",
        item.input_items.as_deref().unwrap_or_default(),
        &mut states,
    );
    states
}
fn response_slice_states(item: &ResponseSnapshot) -> BTreeMap<String, bool> {
    let mut states = BTreeMap::from([
        ("items".into(), item.items.is_none()),
        ("texts".into(), item.texts.is_none()),
        ("reasoning".into(), item.reasoning.is_none()),
        ("reasoning_texts".into(), item.reasoning_texts.is_none()),
        ("thinking_texts".into(), item.thinking_texts.is_none()),
        ("tool_calls".into(), item.tool_calls.is_none()),
    ]);
    item_slice_states(
        "items",
        item.items.as_deref().unwrap_or_default(),
        &mut states,
    );
    states
}
fn check_field_case<T: GoJsonDecode + GoJsonEncode>(
    case: &Value,
    states: fn(&T) -> BTreeMap<String, bool>,
) {
    let decoded = from_go_json::<T>(&unhex(case["input_hex"].as_str().unwrap()));
    assert_eq!(decoded.is_ok(), case["decode_ok"], "{case}");
    if let Ok(item) = decoded {
        assert_eq!(
            serde_json::to_value(states(&item)).unwrap(),
            case["nil_slices"],
            "{case}"
        );
        let encoded = to_go_json_bytes(&item);
        assert_eq!(encoded.is_ok(), case["marshal_ok"], "{case}");
        if let Ok(encoded) = encoded {
            assert_eq!(
                encoded,
                unhex(case["marshal_hex"].as_str().unwrap()),
                "{case}"
            );
        }
    }
}

#[test]
fn snapshot_item_projection_matches_pinned_go_all_discriminants() {
    let fixture = fixture();
    let cases = fixture["snapshot_item_projection_cases"]
        .as_array()
        .unwrap();
    assert_eq!(cases.len(), 25);
    for case in cases {
        let items: Option<Vec<RunItem>> =
            serde_json::from_str(case["input"].as_str().unwrap()).unwrap();
        let output = adk_codec::snapshot_items(items.as_deref().unwrap_or_default());
        assert_eq!(output.is_none(), case["output_nil"]);
        let mut states = BTreeMap::from([("".into(), output.is_none())]);
        item_slice_states("", output.as_deref().unwrap_or_default(), &mut states);
        assert_eq!(
            serde_json::to_value(states).unwrap(),
            case["nil_slices"],
            "{case}"
        );
        assert_eq!(
            adk_codec::snapshots::to_go_json(&output).unwrap(),
            unhex(case["encoded_hex"].as_str().unwrap()),
            "{case}"
        );
    }
}

#[test]
fn field_complete_envelopes_match_pinned_go_bytes_and_slice_state() {
    let fixture = fixture();
    let cases = fixture["envelope_field_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 522);
    for case in cases {
        match case["type"].as_str().unwrap() {
            "LLMRequestSnapshot" => check_field_case(case, request_slice_states),
            "LLMResponseSnapshot" => check_field_case(case, response_slice_states),
            other => panic!("unexpected envelope {other}"),
        }
    }
}

fn item_raw(prefix: &str, items: &[RunItemSnapshot], raw: &mut BTreeMap<String, RawJson>) {
    for (i, item) in items.iter().enumerate() {
        if let Some(call) = &item.tool_call {
            raw.insert(format!("{prefix}/{i}/tool_call"), call.input.clone());
        }
        if let Some(call) = &item.tool_approval {
            raw.insert(format!("{prefix}/{i}/tool_approval"), call.input.clone());
        }
    }
}
fn request_raw(item: &RequestSnapshot) -> BTreeMap<String, RawJson> {
    let mut raw = BTreeMap::new();
    item_raw(
        "input_items",
        item.input_items.as_deref().unwrap_or_default(),
        &mut raw,
    );
    for (i, tool) in item.tools.iter().flatten().enumerate() {
        raw.insert(format!("tools/{i}"), tool.input_schema.clone());
    }
    if let Some(schema) = &item.output_schema {
        raw.insert("output_schema".into(), schema.schema.clone());
    }
    raw
}
fn response_raw(item: &ResponseSnapshot) -> BTreeMap<String, RawJson> {
    let mut raw = BTreeMap::new();
    item_raw("items", item.items.as_deref().unwrap_or_default(), &mut raw);
    for (i, call) in item.tool_calls.iter().flatten().enumerate() {
        raw.insert(format!("tool_calls/{i}"), call.input.clone());
    }
    raw.insert("raw".into(), item.raw.clone());
    raw
}
fn check_decoded<T: GoJsonDecode + GoJsonEncode>(
    case: &Value,
    index: usize,
    collect: impl Fn(&T) -> BTreeMap<String, RawJson>,
) {
    let decoded = from_go_json::<T>(&unhex(case["input_hex"].as_str().unwrap()));
    assert_eq!(
        decoded.is_ok(),
        case["decode_ok"].as_bool().unwrap(),
        "decode case {index}"
    );
    if let Ok(item) = decoded {
        let raw = collect(&item);
        let expected = case["raw"].as_object().unwrap();
        assert_eq!(raw.len(), expected.len(), "raw case {index}");
        for (path, value) in &raw {
            assert_eq!(
                !value.is_missing(),
                expected[path]["present"].as_bool().unwrap(),
                "raw presence {index}/{path}"
            );
            assert!(
                value.bytes() == unhex(expected[path]["hex"].as_str().unwrap()),
                "raw bytes {index}/{path}"
            );
        }
        let encoded = to_go_json_bytes(&item);
        assert_eq!(
            encoded.is_ok(),
            case["marshal_ok"].as_bool().unwrap(),
            "marshal case {index}"
        );
        if let Ok(bytes) = encoded {
            assert!(
                bytes == unhex(case["marshal_hex"].as_str().unwrap()),
                "encoded case {index}"
            );
        }
        assert_eq!(collect(&item), raw, "encoding mutated case {index}");
    }
}
#[test]
fn envelope_byte_decoding_matches_go_without_mutating_raw() {
    let fixture = fixture();
    let cases = fixture["envelope_byte_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 130);
    for (index, case) in cases.iter().enumerate() {
        match case["type"].as_str().unwrap() {
            "request" => check_decoded::<RequestSnapshot>(case, index, request_raw),
            "response" => check_decoded::<ResponseSnapshot>(case, index, response_raw),
            other => panic!("unexpected envelope {other}"),
        }
    }
}
#[test]
fn envelope_direct_raw_encoding_matches_go_depth_and_omission() {
    let fixture = fixture();
    let cases = fixture["envelope_encode_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 80);
    for (index, case) in cases.iter().enumerate() {
        let raw = if case["raw_nil"].as_bool().unwrap() {
            RawJson::Missing
        } else {
            RawJson::EncodedBytes(unhex(case["raw_hex"].as_str().unwrap()))
        };
        let encoded = match case["location"].as_str().unwrap() {
            "request_tool" => to_go_json_bytes(&RequestSnapshot {
                tools: Some(vec![ToolSnapshot {
                    input_schema: raw,
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            "request_schema" => to_go_json_bytes(&RequestSnapshot {
                output_schema: Some(OutputSchema {
                    schema: raw,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            "request_call" => to_go_json_bytes(&RequestSnapshot {
                input_items: Some(vec![RunItemSnapshot {
                    tool_call: Some(ToolCallSnapshot {
                        input: raw,
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            "request_approval" => to_go_json_bytes(&RequestSnapshot {
                input_items: Some(vec![RunItemSnapshot {
                    tool_approval: Some(ToolApprovalSnapshot {
                        input: raw,
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            "response_raw" => to_go_json_bytes(&ResponseSnapshot {
                raw,
                ..Default::default()
            }),
            "response_call" => to_go_json_bytes(&ResponseSnapshot {
                tool_calls: Some(vec![ToolCallSnapshot {
                    input: raw,
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            "response_item_call" => to_go_json_bytes(&ResponseSnapshot {
                items: Some(vec![RunItemSnapshot {
                    tool_call: Some(ToolCallSnapshot {
                        input: raw,
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            "response_approval" => to_go_json_bytes(&ResponseSnapshot {
                items: Some(vec![RunItemSnapshot {
                    tool_approval: Some(ToolApprovalSnapshot {
                        input: raw,
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            other => panic!("unexpected raw location {other}"),
        };
        assert_eq!(
            encoded.is_ok(),
            case["marshal_ok"].as_bool().unwrap(),
            "encode case {index}"
        );
        if let Ok(bytes) = encoded {
            assert!(
                bytes == unhex(case["marshal_hex"].as_str().unwrap()),
                "encoded case {index}"
            );
        }
    }
}
