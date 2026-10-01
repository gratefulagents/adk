use adk_codec::snapshots::{ModelSettings, RequestSnapshot};
use serde_json::Value;

#[test]
fn pinned_writer_request_bytes_round_trip_without_reordering_fields() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    let bytes = fixture["request_json"].as_str().unwrap();
    let snapshot: RequestSnapshot = serde_json::from_str(bytes).unwrap();
    assert_eq!(snapshot.agent_name, "agent");
    assert_eq!(serde_json::to_string(&snapshot).unwrap(), bytes);
}

#[test]
fn nil_and_explicit_zero_settings_are_distinct() {
    let snapshot = RequestSnapshot::default();
    assert_eq!(
        serde_json::to_string(&snapshot).unwrap(),
        r#"{"settings":{}}"#
    );
    let settings = ModelSettings {
        temperature: Some(0.0),
        parallel_tool_calls: Some(false),
        ..Default::default()
    };
    assert_eq!(
        serde_json::to_value(settings).unwrap(),
        serde_json::json!({"temperature":0.0,"parallel_tool_calls":false})
    );
}

#[test]
fn float_thresholds_and_html_escaping_match_pinned_go_bytes() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    for expected in fixture["request_variants"].as_array().unwrap() {
        let expected = expected.as_str().unwrap();
        let snapshot: RequestSnapshot = serde_json::from_str(expected).unwrap();
        assert_eq!(
            adk_codec::snapshots::to_go_json(&snapshot).unwrap(),
            expected.as_bytes()
        );
    }
    for temperature in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let snapshot = RequestSnapshot {
            settings: ModelSettings {
                temperature: Some(temperature),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(adk_codec::snapshots::to_go_json(&snapshot).is_err());
    }
}

#[test]
fn json_null_scalars_and_collections_follow_go_zero_values() {
    let snapshot: RequestSnapshot = serde_json::from_str(r#"{"agent_name":null,"input_items":null,"tools":null,"settings":{"max_tokens":null,"parallel_tool_calls":null,"stop_sequences":null}}"#).unwrap();
    assert_eq!(snapshot, RequestSnapshot::default());
}

#[test]
fn raw_json_preserves_order_duplicate_keys_and_numbers_but_compacts_and_escapes_like_go() {
    use adk_codec::snapshots::{ToolSnapshot, to_go_json};
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    let schema = serde_json::from_str(fixture["raw_input_schema"].as_str().unwrap()).unwrap();
    let snapshot = RequestSnapshot {
        tools: vec![ToolSnapshot {
            name: "raw".into(),
            input_schema: schema,
            ..Default::default()
        }],
        ..Default::default()
    };
    let expected = fixture["raw_request_json"].as_str().unwrap();
    assert_eq!(to_go_json(&snapshot).unwrap(), expected.as_bytes());
}

#[test]
fn raw_json_null_remains_distinct_from_missing_schema() {
    let explicit = r#"{"tools":[{"name":"raw","input_schema":null,"read_only":false,"needs_approval":false}],"settings":{},"output_schema":{"name":"output","schema":null,"strict":false}}"#;
    let snapshot: RequestSnapshot = serde_json::from_str(explicit).unwrap();
    assert!(!snapshot.tools[0].input_schema.is_missing());
    assert_eq!(
        adk_codec::snapshots::to_go_json(&snapshot).unwrap(),
        explicit.as_bytes()
    );
}
