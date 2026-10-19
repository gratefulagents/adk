use adk_providers::anthropic::StreamAssembler;
use serde_json::Value;

#[test]
fn incremental_assembler_matches_pinned_go_at_every_event() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/anthropic-assembler/observations.json"
    ))
    .unwrap();
    let cases = fixture["cases"].as_object().unwrap();
    assert_eq!(cases.len(), 16);
    for (name, case) in cases {
        let mut assembler = StreamAssembler::new();
        let initial = assembler.response().unwrap();
        let mut retained = Vec::new();
        for (index, expected) in case["states"].as_array().unwrap().iter().enumerate() {
            if index > 0 {
                assembler
                    .add(case["events"][index - 1].as_str().unwrap())
                    .unwrap();
            }
            let actual = assembler.response();
            assert_eq!(
                actual.is_ok(),
                expected["ok"].as_bool().unwrap(),
                "{name}/{index}"
            );
            if let Ok(document) = actual {
                assert_eq!(
                    document.as_str(),
                    expected["json"].as_str().unwrap(),
                    "{name}/{index}"
                );
                retained.push((document, expected["json"].as_str().unwrap()));
            }
        }
        assert_eq!(
            initial.as_str(),
            case["states"][0]["json"].as_str().unwrap()
        );
        for (document, expected) in retained {
            assert_eq!(document.as_str(), expected, "owned snapshot {name}");
        }
    }
}

#[test]
fn encrypted_compaction_assembles_without_a_message_start() {
    let mut assembler = StreamAssembler::new();
    for event in [
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"compaction","id":"comp_1"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"compaction_encrypted_content","encrypted_content":"opaque"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
    ] {
        assembler.add(event).unwrap();
    }
    let response: Value = serde_json::from_str(assembler.response().unwrap().as_str()).unwrap();
    assert_eq!(response["content"].as_array().unwrap().len(), 1);
    assert_eq!(response["content"][0]["encrypted_content"], "opaque");
}

#[test]
fn malformed_event_does_not_mutate_partial_state() {
    let mut assembler = StreamAssembler::new();
    assembler
        .add(r#"{"type":"message_start","message":{"id":"kept"}}"#)
        .unwrap();
    let before = assembler.response().unwrap();
    for event in [
        "{",
        r#"{"type":"message_start","message":{"id":123}}"#,
        r#"{"type":"content_block_start","index":1.5}"#,
    ] {
        assert!(assembler.add(event).is_err());
        assert_eq!(assembler.response().unwrap(), before);
    }
}
