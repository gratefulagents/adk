use adk_codec::{dto::*, replay, schema, timestamp::GoTimestamp};
use serde_json::{Value, json};

#[test]
fn all_sdk_fixture_transforms() {
    let fixture: Value = serde_json::from_str(include_str!("../../../fixtures/sdk.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 6);
    for case in cases {
        let actual = replay(case["operation"].as_str().unwrap(), &case["input"]).unwrap();
        assert_eq!(actual, case["expected"], "{}", case["name"]);
    }
}

#[test]
fn go_run_item_wire_roundtrip_is_typed() {
    let fixture: Value = serde_json::from_str(include_str!("../../../fixtures/sdk.json")).unwrap();
    let input = &fixture["cases"][0]["input"];
    let items: Vec<RunItem> = serde_json::from_value(input.clone()).unwrap();
    assert_eq!(items[0].kind, RunItemType(5));
    assert_eq!(items[1].tool_call.as_ref().unwrap().name, "Read");
    assert_eq!(serde_json::to_value(items).unwrap(), *input);
    let wire = &fixture["cases"][1]["input"];
    let response: ModelResponse = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(response.end_turn, Some(false));
    assert_eq!(serde_json::to_value(response).unwrap(), *wire);
}

#[test]
fn go_snapshot_and_child_wire_roundtrips_are_typed() {
    let fixture: Value = serde_json::from_str(include_str!("../../../fixtures/sdk.json")).unwrap();
    for index in [1, 2] {
        let expected = &fixture["cases"][index]["expected"];
        let typed: ResponseSnapshot = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), *expected);
    }
    for index in [3, 4] {
        let expected = &fixture["cases"][index]["expected"];
        let typed: ChildToolEvent = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), *expected);
    }
}

#[test]
fn absent_null_false_and_empty_follow_go_field_rules() {
    for input in [Value::Null, json!([])] {
        assert_eq!(replay("snapshot_items", &input).unwrap(), Value::Null);
    }
    assert_eq!(
        replay("response_snapshot", &Value::Null).unwrap(),
        Value::Null
    );
    for end in [
        None,
        Some(Value::Null),
        Some(json!(false)),
        Some(json!(true)),
    ] {
        let mut input = json!({});
        if let Some(v) = &end {
            input["EndTurn"] = v.clone();
        }
        let out = replay("response_snapshot", &input).unwrap();
        assert_eq!(out.get("end_turn"), end.as_ref().filter(|v| !v.is_null()));
        assert_eq!(
            out["usage"],
            json!({"requests":0,"input_tokens":0,"output_tokens":0})
        );
        assert_eq!(out["raw_available"], false);
        assert!(out.get("raw").is_none());
        assert!(out.get("items").is_none());
    }
    let nil = replay("snapshot_items", &json!([{"Type":1,"ToolCall":{}}])).unwrap();
    let null = replay(
        "snapshot_items",
        &json!([{"Type":1,"ToolCall":{"input":null}}]),
    )
    .unwrap();
    assert_eq!(nil, json!([{"type":"tool_call","tool_call":{}}]));
    assert_eq!(
        null,
        json!([{"type":"tool_call","tool_call":{"input":null}}])
    );
    let zeros = replay(
        "snapshot_items",
        &json!([{"Message":{"text":null,"phase":null,"images":null},"Type":null}]),
    )
    .unwrap();
    assert_eq!(zeros, json!([{"type":"message"}]));
    assert_eq!(
        replay("response_snapshot", &json!({"Raw":false})).unwrap()["raw"],
        false
    );
}

#[test]
fn enum_numeric_wire_string_snapshot_and_unknown_go_behavior() {
    let kinds = [
        "message",
        "tool_call",
        "tool_output",
        "handoff_call",
        "handoff_output",
        "reasoning",
        "tool_approval",
        "compaction",
    ];
    for (index, kind) in kinds.iter().enumerate() {
        let result = replay("snapshot_items", &json!([{"Type":index}])).unwrap();
        assert_eq!(result, json!([{"type":kind}]));
    }
    assert_eq!(
        replay("snapshot_items", &json!([{"Type":99}])).unwrap(),
        json!([{"type":"unknown"}])
    );
    for bad in [
        json!("reasoning"),
        json!(1.5),
        json!(true),
        json!(9223372036854775808u64),
    ] {
        assert!(replay("snapshot_items", &json!([{"Type":bad}])).is_err());
    }
    assert!(serde_json::from_value::<RunItemSnapshot>(json!({"type":5})).is_err());
}

#[test]
fn every_payload_uses_go_omission_and_mapping() {
    let input = json!([
        {"Type":0,"Agent":{"Name":"研究者"},"Message":{"text":"hi","phase":"commentary","images":[{"media_type":"image/png","data":"YWJj"}]}},
        {"Type":2,"ToolOutput":{"call_id":"c","content":"ok","is_error":false}},
        {"Type":3,"HandoffCall":{"from_agent":"a","to_agent":"b"}},
        {"Type":4,"HandoffOutput":{"from_agent":"a","to_agent":"b"}},
        {"Type":5,"Reasoning":{"text":"考える","redacted_data":"opaque"}},
        {"Type":6,"ToolApproval":{"approved":false}},
        {"Type":7,"Compaction":{"content":"summary","created_by":"model"}}
    ]);
    let actual = replay("snapshot_items", &input).unwrap();
    assert_eq!(
        actual,
        json!([
            {"type":"message","agent_name":"研究者","message_text":"hi","message_phase":"commentary","message_images":[{"media_type":"image/png","data":"YWJj"}]},
            {"type":"tool_output","tool_output":{"call_id":"c","content":"ok"}},
            {"type":"handoff_call","handoff_call":{"from_agent":"a","to_agent":"b"}},
            {"type":"handoff_output","handoff_output":{"from_agent":"a","to_agent":"b"}},
            {"type":"reasoning","reasoning_text":"考える","thinking_text":"考える","reasoning":{"text":"考える","thinking":"考える","redacted_data":"opaque"}},
            {"type":"tool_approval","tool_approval":{"approved":false}},
            {"type":"compaction","compaction":{"content":"summary","created_by":"model"}}
        ])
    );
}

#[test]
fn exact_integer_and_arbitrary_json_number_precision() {
    let source = r#"{"Items":[{"Type":1,"ToolCall":{"id":"c","input":{"large":123456789012345678901234567890,"decimal":0.12345678901234567890123456789}}}],"Usage":{"input_tokens":9223372036854775807},"Raw":{"n":9007199254740993}}"#;
    let input: Value = serde_json::from_str(source).unwrap();
    let out = replay("response_snapshot", &input).unwrap();
    assert_eq!(
        out["usage"]["input_tokens"].to_string(),
        "9223372036854775807"
    );
    assert_eq!(
        out["tool_calls"][0]["input"],
        input["Items"][0]["ToolCall"]["input"]
    );
    assert_eq!(out["raw"]["n"].to_string(), "9007199254740993");
    assert_eq!(
        out["tool_calls"][0]["input"]["decimal"].to_string(),
        "0.12345678901234567890123456789"
    );
    assert!(
        replay(
            "response_snapshot",
            &json!({"Usage":{"input_tokens":9223372036854775808u64}})
        )
        .is_err()
    );
}

#[test]
fn timestamp_nanoseconds_offsets_zero_and_invalid_calendar() {
    for (input, expected) in [
        (
            "2000-01-01T00:00:00.000000001Z",
            "2000-01-01T00:00:00.000000001Z",
        ),
        (
            "2024-02-29T12:34:56.120000000+05:30",
            "2024-02-29T12:34:56.12+05:30",
        ),
        ("2024-02-29T12:34:56.000+00:00", "2024-02-29T12:34:56Z"),
    ] {
        let stamp: GoTimestamp = serde_json::from_value(json!(input)).unwrap();
        assert_eq!(serde_json::to_value(stamp).unwrap(), expected);
    }
    let event: ContentEvent = serde_json::from_value(json!({"ts":null})).unwrap();
    assert_eq!(
        serde_json::to_value(event).unwrap()["ts"],
        "0001-01-01T00:00:00Z"
    );
    for bad in [
        "2023-02-29T00:00:00Z",
        "2024-01-01T00:00:60Z",
        "2024-01-01T00:00:00.1234567890Z",
        "2024-01-01T00:00:00+24:00",
        "not-time",
        "💥",
    ] {
        assert!(serde_json::from_value::<GoTimestamp>(json!(bad)).is_err());
    }
}

#[test]
fn child_filtering_and_start_end_fields() {
    for input in [
        json!({"type":"tool_start"}),
        json!({"type":"message","parent_call_id":"p"}),
    ] {
        assert_eq!(replay("child_event", &input).unwrap(), Value::Null);
    }
    let out = replay("child_event", &json!({"type":"tool_start","parent_call_id":"p","input_raw":"🙂","output":"ignored","is_error":true,"tool_duration_ms":5})).unwrap();
    assert_eq!(out["InputRaw"], "🙂");
    assert_eq!(out["Output"], "");
    assert_eq!(out["DurationMS"], 0);
    assert_eq!(out["IsError"], false);
}

#[test]
fn typed_schemas_describe_go_names_enum_precision_and_utf8() {
    let run = serde_json::to_value(schema::<RunItem>()).unwrap();
    assert!(run["properties"].get("Type").is_some());
    assert!(run["properties"].get("ToolCall").is_some());
    assert!(run["properties"].get("type").is_none());
    let kind = serde_json::to_value(schema::<RunItemType>()).unwrap();
    assert_eq!(kind["type"], "integer");
    let snapshot_kind = serde_json::to_value(schema::<SnapshotType>()).unwrap();
    assert_eq!(snapshot_kind["type"], "string");
    assert!(
        snapshot_kind["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("reasoning"))
    );
    assert_eq!(
        serde_json::to_value(schema::<GoTimestamp>()).unwrap()["format"],
        "date-time"
    );
    let text = "日本語 — café e\u{301} 🙂\n\u{0000}";
    let out = replay(
        "snapshot_items",
        &json!([{"Type":0,"Message":{"text":text}}]),
    )
    .unwrap();
    let typed: Vec<RunItemSnapshot> = serde_json::from_value(out.clone()).unwrap();
    assert_eq!(typed[0].message_text, text);
    assert_eq!(serde_json::to_value(typed).unwrap(), out);
    let schema = schema::<RunItemSnapshot>();
    assert!(serde_json::from_str::<Value>(&serde_json::to_string(&schema).unwrap()).is_ok());
}

#[test]
fn state_rejects_order_dependencies_and_unknown_events() {
    let fixture: Value = serde_json::from_str(include_str!("../../../fixtures/sdk.json")).unwrap();
    let original = &fixture["cases"][5]["input"];
    let mut wrong = original.clone();
    wrong[1]["seq"] = json!(8);
    assert!(replay("state_ready", &wrong).is_err());
    let mut wrong = original.clone();
    wrong[2]["payload"]["depends_on"] = json!(["missing"]);
    assert!(replay("state_ready", &wrong).is_err());
    let mut wrong = original.clone();
    wrong[1]["type"] = json!("task.unknown");
    assert!(replay("state_ready", &wrong).is_err());
    assert!(replay("persist_transcript", &json!({})).is_err());
}

#[test]
fn go_float_cost_wire_encoding() {
    for (value, expected) in [
        (0.0, "0"),
        (-0.0, "0"),
        (1.0, "1"),
        (0.25, "0.25"),
        (1e-6, "0.000001"),
        (1e-7, "1e-7"),
        (1e20, "100000000000000000000"),
        (1e21, "1e+21"),
    ] {
        let response = ModelResponse {
            cost_usd: value,
            ..Default::default()
        };
        let wire = serde_json::to_value(response).unwrap();
        assert_eq!(wire["CostUSD"].to_string(), expected);
    }
    assert!(
        serde_json::to_value(ModelResponse {
            cost_usd: f64::NAN,
            ..Default::default()
        })
        .is_err()
    );
}

#[test]
fn state_ready_sorts_equal_seconds_by_exact_nanoseconds() {
    let create = |seq, id: &str, stamp: &str| json!({"seq":seq,"event_id":id,"type":"task.created","payload":{"id":id,"status":"open","priority":1,"updated_at":stamp}});
    let events = json!([
        create(1, "later", "2000-01-01T00:00:00.000000002Z"),
        create(2, "whole", "2000-01-01T00:00:00Z"),
        create(3, "first", "2000-01-01T00:00:00.000000001Z")
    ]);
    assert_eq!(
        replay("state_ready", &events).unwrap(),
        json!(["later", "first", "whole"])
    );
}
