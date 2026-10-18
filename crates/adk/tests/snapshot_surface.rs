#![cfg(feature = "compat")]

use adk::codec::{
    dto::{CompactionSnapshot, ReasoningSnapshot, ToolApprovalSnapshot, ToolCallSnapshot},
    from_go_json, to_go_json_bytes,
};

fn unhex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
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
