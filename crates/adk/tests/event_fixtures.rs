#![cfg(feature = "observability")]

use adk::observability::{EventRecord, LineDecoder};

const FIXTURE: &[u8] = include_bytes!("../../../fixtures/observability/native-v1.jsonl");

#[test]
fn versioned_native_fixture_preserves_bytes_and_fragment_order() {
    let mut decoder = LineDecoder::new(4096).unwrap();
    let mut records = Vec::new();
    for chunk in FIXTURE.chunks(7) {
        records.extend(decoder.push(chunk).into_iter().map(Result::unwrap));
    }
    assert!(decoder.finish().is_none());
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].sequence, 1);
    assert_eq!(records[1].sequence, 2);
    assert_eq!(records[1].progress.usage.input_tokens, 4);
    assert_eq!(records[1].progress.cost, 0.5);
    let roundtrip: Vec<_> = records
        .iter()
        .flat_map(|record| record.to_json_line().unwrap())
        .collect();
    assert_eq!(roundtrip, FIXTURE);
    let mut unknown = serde_json::to_value(&records[0]).unwrap();
    unknown["schema_version"] = 99.into();
    assert!(EventRecord::from_json_line(&serde_json::to_vec(&unknown).unwrap()).is_err());
}
