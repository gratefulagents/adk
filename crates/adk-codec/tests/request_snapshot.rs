use adk_codec::snapshots::{ModelSettings, RequestSnapshot};
use serde_json::Value;

#[test]
fn repeated_snapshot_images_retain_hidden_nonempty_slots() {
    let snapshot: RequestSnapshot = serde_json::from_str(
        r#"{"input_items":[{"message_images":[{"media_type":"image/png","data":"one"},{"media_type":"image/jpeg","data":"two"}],"MESSAGE_IMAGES":[{}],"message_images":[null,null]}]}"#,
    ).unwrap();
    let images = &snapshot.input_items[0].message_images;
    assert_eq!(images.len(), 2);
    assert_eq!(images[0].media_type, "image/png");
    assert_eq!(images[0].data, "one");
    assert_eq!(images[1].media_type, "image/jpeg");
    assert_eq!(images[1].data, "two");
}

#[test]
fn request_snapshot_field_records_match_independent_pinned_go() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    for case in fixture["request_record_cases"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let decoded = serde_json::from_str::<RequestSnapshot>(input);
        assert_eq!(
            decoded.is_ok(),
            case["accepted"].as_bool().unwrap(),
            "{input}: {decoded:?}"
        );
        if let Ok(snapshot) = decoded {
            assert_eq!(
                adk_codec::snapshots::to_go_json(&snapshot).unwrap(),
                case["encoded"].as_str().unwrap().as_bytes(),
                "{input}"
            );
        }
    }
}

#[test]
fn model_settings_merge_preserves_original_sdk_example() {
    let base = ModelSettings {
        temperature: Some(0.5),
        max_tokens: 1000,
        ..Default::default()
    };
    let overrides = ModelSettings {
        max_tokens: 2000,
        reasoning_effort: "high".into(),
        text_verbosity: "low".into(),
        ..Default::default()
    };
    let merged = base.merge(&overrides);
    assert_eq!(merged.temperature, Some(0.5));
    assert_eq!(merged.max_tokens, 2000);
    assert_eq!(merged.reasoning_effort, "high");
    assert_eq!(merged.text_verbosity, "low");
}

#[test]
fn model_settings_merge_matches_pinned_sdk_without_aliasing_inputs() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    for case in fixture["merged_settings_cases"].as_array().unwrap() {
        let mut base: ModelSettings = serde_json::from_str(case["base"].as_str().unwrap()).unwrap();
        let mut overrides: ModelSettings =
            serde_json::from_str(case["override"].as_str().unwrap()).unwrap();
        let original_base = base.clone();
        let original_overrides = overrides.clone();
        let merged = base.merge(&overrides);
        assert_eq!(base, original_base);
        assert_eq!(overrides, original_overrides);
        assert_eq!(
            adk_codec::snapshots::to_go_json(&merged).unwrap(),
            case["merged"].as_str().unwrap().as_bytes(),
            "{case}"
        );
        base.stop_sequences
            .get_or_insert_with(Vec::new)
            .push("changed base".into());
        overrides
            .stop_sequences
            .get_or_insert_with(Vec::new)
            .push("changed override".into());
        assert_eq!(
            adk_codec::snapshots::to_go_json(&merged).unwrap(),
            case["merged"].as_str().unwrap().as_bytes()
        );
    }
}

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
        serde_json::json!({"temperature":0,"parallel_tool_calls":false})
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

#[test]
fn snapshot_schema_case_folded_null_updates_preserve_previous_values() {
    let schema: adk_codec::snapshots::OutputSchema =
        serde_json::from_str(r#"{"NAME":"first","strict":true,"name":null,"STRICT":null}"#)
            .unwrap();
    assert_eq!(schema.name, "first");
    assert!(schema.strict);
}

#[test]
fn settings_folded_scalar_null_and_slice_reuse() {
    let settings: adk_codec::snapshots::ModelSettings = serde_json::from_str(
        r#"{"max_tokens":7,"MAX_TOKENS":null,"temperature":0.5,"TEMPERATURE":null,"stop_sequences":["a","b"],"STOP_SEQUENCES":[null],"stop_sequences":[null,null]}"#,
    ).unwrap();
    assert_eq!(settings.max_tokens, 7);
    assert_eq!(settings.temperature, None);
    assert_eq!(settings.stop_sequences, Some(vec!["a".into(), "b".into()]));
}

#[test]
fn snapshot_settings_preserve_nil_and_allocated_empty_sequences() {
    let nil: RequestSnapshot =
        adk_codec::from_go_json(br#"{"settings":{"stop_sequences":null}}"#).unwrap();
    let empty: RequestSnapshot =
        adk_codec::from_go_json(br#"{"settings":{"stop_sequences":[]}}"#).unwrap();
    assert_eq!(nil.settings.stop_sequences, None);
    assert_eq!(empty.settings.stop_sequences, Some(vec![]));
    assert_eq!(
        empty
            .settings
            .merge(&ModelSettings::default())
            .stop_sequences,
        Some(vec![])
    );
    assert_eq!(
        adk_codec::to_go_json_bytes(&nil).unwrap(),
        adk_codec::to_go_json_bytes(&empty).unwrap()
    );
}
