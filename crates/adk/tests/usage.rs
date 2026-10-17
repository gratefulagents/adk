#![cfg(feature = "compat")]

use adk::codec::dto::Usage;
use serde_json::Value;

#[test]
fn facade_signed_usage_matches_pinned_go() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/usage/observations.json")).unwrap();
    assert_eq!(
        fixture["sdk_revision"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 64);
    for case in cases {
        let mut sum: Usage = serde_json::from_value(case["left"].clone()).unwrap();
        let right: Usage = serde_json::from_value(case["right"].clone()).unwrap();
        sum.add(&right);
        assert_eq!(serde_json::to_value(&sum).unwrap(), case["sum"]);
        assert_eq!(sum.total_tokens(), case["total"].as_i64().unwrap());
        assert_eq!(serde_json::to_value(right).unwrap(), case["right"]);
    }
}
