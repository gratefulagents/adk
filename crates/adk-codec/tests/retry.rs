use adk_codec::retry::*;
use serde_json::Value;

#[test]
fn legacy_retry_backoff_matches_pinned_go() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/retry/observations.json")).unwrap();
    assert_eq!(
        fixture["provenance"]["commit"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 1890);
    for case in fixture["cases"].as_array().unwrap() {
        let policy = RetryPolicy {
            max_retries: -3,
            backoff: RetryBackoffSettings {
                initial_delay_ms: case["initial"].as_i64().unwrap(),
                max_delay_ms: case["maximum"].as_i64().unwrap(),
                multiplier: case["multiplier"].as_str().unwrap().parse().unwrap(),
            },
        };
        assert_eq!(
            policy.delay_for_attempt(case["attempt"].as_i64().unwrap()),
            case["delay"].as_i64().unwrap(),
            "{case}"
        );
        assert_eq!(policy.max_retries, -3);
    }
    let policy = default_retry_policy();
    assert_eq!(policy.max_retries, fixture["default"]["max_retries"]);
    assert_eq!(
        policy.backoff.initial_delay_ms,
        fixture["default"]["initial"]
    );
    assert_eq!(policy.backoff.max_delay_ms, fixture["default"]["maximum"]);
    assert_eq!(
        policy.backoff.multiplier,
        fixture["default"]["multiplier"].as_f64().unwrap()
    );
    assert_eq!(policy.delay_for_attempt(2), fixture["default"]["attempt_2"]);
    assert_eq!(RetryPolicy::default().max_retries, 0);
    assert_eq!(RetryPolicy::default().delay_for_attempt(5), 0);
}
