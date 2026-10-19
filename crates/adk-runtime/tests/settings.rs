use adk_runtime::settings::{reasoning_settings, routing_settings, verbosity_settings};
use serde_json::Value;

#[test]
fn routing_labels_and_budgets_match_independently_executed_sdk_helpers() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    let cases = fixture["routing_settings"]["labels"].as_array().unwrap();
    assert_eq!(cases.len(), 72);
    for case in cases {
        let reasoning = case["reasoning"].as_str().unwrap();
        let verbosity = case["verbosity"].as_str().unwrap();
        let combined = routing_settings(reasoning, verbosity);
        assert_eq!(
            Value::Object(combined.clone()),
            case["settings"],
            "{reasoning:?}/{verbosity:?}"
        );
        let mut separate = reasoning_settings(reasoning);
        separate.extend(verbosity_settings(verbosity));
        assert_eq!(separate, combined);
    }
}

#[test]
fn original_mode_reasoning_settings_regression() {
    for (level, budget) in [
        ("none", 0),
        ("minimal", 1024),
        ("low", 2048),
        ("medium", 4096),
        ("high", 8192),
        ("xhigh", 16384),
        ("max", 24576),
    ] {
        let settings = reasoning_settings(level);
        assert_eq!(
            settings
                .get("thinking_budget")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            budget,
            "{level}"
        );
        assert_eq!(settings["reasoning_effort"], level);
    }
}

#[test]
fn named_mode_labels_flow_into_native_routing_settings() {
    use adk_codec::dto::{ModeReasoningLevel, ModeTextVerbosity};
    let settings = routing_settings(&ModeReasoningLevel::XHIGH, &ModeTextVerbosity::LOW);
    assert_eq!(settings["thinking_budget"], 16384);
    assert_eq!(settings["reasoning_effort"], "xhigh");
    assert_eq!(settings["text_verbosity"], "low");
    let unknown = ModeReasoningLevel(" not-a-tier ".into());
    assert!(reasoning_settings(&unknown).is_empty());
    assert_eq!(unknown.to_string(), " not-a-tier ");
}
