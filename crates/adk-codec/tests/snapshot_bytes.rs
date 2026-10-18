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
    item_raw("input_items", &item.input_items, &mut raw);
    for (i, tool) in item.tools.iter().enumerate() {
        raw.insert(format!("tools/{i}"), tool.input_schema.clone());
    }
    if let Some(schema) = &item.output_schema {
        raw.insert("output_schema".into(), schema.schema.clone());
    }
    raw
}
fn response_raw(item: &ResponseSnapshot) -> BTreeMap<String, RawJson> {
    let mut raw = BTreeMap::new();
    item_raw("items", &item.items, &mut raw);
    for (i, call) in item.tool_calls.iter().enumerate() {
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
                tools: vec![ToolSnapshot {
                    input_schema: raw,
                    ..Default::default()
                }],
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
                input_items: vec![RunItemSnapshot {
                    tool_call: Some(ToolCallSnapshot {
                        input: raw,
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            "request_approval" => to_go_json_bytes(&RequestSnapshot {
                input_items: vec![RunItemSnapshot {
                    tool_approval: Some(ToolApprovalSnapshot {
                        input: raw,
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            "response_raw" => to_go_json_bytes(&ResponseSnapshot {
                raw,
                ..Default::default()
            }),
            "response_call" => to_go_json_bytes(&ResponseSnapshot {
                tool_calls: vec![ToolCallSnapshot {
                    input: raw,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            "response_item_call" => to_go_json_bytes(&ResponseSnapshot {
                items: vec![RunItemSnapshot {
                    tool_call: Some(ToolCallSnapshot {
                        input: raw,
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            "response_approval" => to_go_json_bytes(&ResponseSnapshot {
                items: vec![RunItemSnapshot {
                    tool_approval: Some(ToolApprovalSnapshot {
                        input: raw,
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
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
