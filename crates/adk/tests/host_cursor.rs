#![cfg(feature = "host")]

use adk::{codec, host::Cursor};

#[test]
fn cursor_json_matches_pinned_host_record() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/host-loop/sdk-cursor.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 41);
    for case in cases {
        let input: Vec<u8> = case["input_hex"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        let decoded = codec::from_go_json::<Cursor>(&input);
        assert_eq!(
            decoded.is_ok(),
            case["accepted"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        if let Ok(cursor) = decoded {
            assert_eq!(cursor.message_id, case["message_id"].as_i64().unwrap());
            assert_eq!(cursor.token, case["token"].as_str().unwrap());
            assert_eq!(
                codec::to_go_json_bytes(&cursor).unwrap(),
                case["wire_json"].as_str().unwrap().as_bytes()
            );
        }
    }
}

#[test]
fn cursor_serde_roundtrip_and_in_place_nulls_preserve_persistence_state() {
    use serde::Deserialize;

    assert_eq!(serde_json::to_string(&Cursor::default()).unwrap(), "{}");
    let mut cursor = Cursor {
        message_id: i64::MIN,
        token: "resume".into(),
    };
    let encoded = serde_json::to_vec(&cursor).unwrap();
    assert_eq!(serde_json::from_slice::<Cursor>(&encoded).unwrap(), cursor);
    let mut decoder = serde_json::Deserializer::from_str(r#"{"MESSAGE_ID":null,"TOKEN":null}"#);
    Cursor::deserialize_in_place(&mut decoder, &mut cursor).unwrap();
    assert_eq!(cursor.message_id, i64::MIN);
    assert_eq!(cursor.token, "resume");
    let mut decoder = serde_json::Deserializer::from_str(r#"{"message_id":0,"token":""}"#);
    Cursor::deserialize_in_place(&mut decoder, &mut cursor).unwrap();
    assert_eq!(cursor, Cursor::default());
}
