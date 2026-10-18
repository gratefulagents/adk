use adk_core::*;
use serde_json::Value;

#[test]
fn action_authorization_normalization_matches_pinned_go() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/action-authorization/observations.json"
    ))
    .unwrap();
    assert_eq!(
        fixture["provenance"]["commit"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    assert_eq!(fixture["normalizations"].as_array().unwrap().len(), 80);
    for case in fixture["normalizations"].as_array().unwrap() {
        let input: ActionAuthorization = serde_json::from_value(case["input"].clone()).unwrap();
        let error = case["failed"]
            .as_bool()
            .unwrap()
            .then(|| Error::new(ErrorCategory::Host, "private error not exposed to model"));
        let output = normalize_action_authorization(input, error.as_ref());
        assert_eq!(
            serde_json::to_value(output).unwrap(),
            case["output"],
            "{case}"
        );
    }
    assert_eq!(
        [
            ACTION_DECISION_ALLOW,
            ACTION_DECISION_DENY,
            ACTION_DECISION_ASK,
            ACTION_DECISION_CLASSIFY
        ],
        ["allow", "deny", "ask", "classify"]
    );
    assert_eq!(
        [ACTION_RISK_LOW, ACTION_RISK_MEDIUM, ACTION_RISK_HIGH],
        ["low", "medium", "high"]
    );
    let failure: ActionAuthorizationFailure = Error::new(ErrorCategory::Host, "failed").into();
    assert!(failure.risk.is_empty());
    assert_eq!(failure.to_string(), "failed");
}
