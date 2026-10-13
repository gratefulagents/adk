use adk_core::{Content, RunItem, ToolCall, ToolOutput};
use adk_runtime::autoloop::{AutoTracker, DEFAULT_MAX_AUTO_LOOPS, build_smart_nudge};
use serde_json::{Value, json};

#[test]
fn autonomous_progress_matches_pinned_public_sdk() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/autoloop/observations.json")).unwrap();
    assert_eq!(
        json!(DEFAULT_MAX_AUTO_LOOPS),
        fixture["default_max_auto_loops"]
    );
    assert_eq!(
        build_smart_nudge(None, "plan"),
        fixture["nil_nudge"].as_str().unwrap()
    );
    for case in fixture["cases"].as_array().unwrap() {
        let mut tracker = AutoTracker::default();
        let input = &case["input"];
        let state = |tracker: &AutoTracker| {
            let breaker = tracker.check_circuit_breakers();
            json!({
                "no_tool_turns": tracker.consecutive_no_tool_turns(),
                "tool_count": tracker.tool_call_count(),
                "tripped": breaker.tripped,
                "reason": breaker.reason,
                "nudge": build_smart_nudge(Some(tracker), input["phase"].as_str().unwrap()),
            })
        };
        let mut states = vec![state(&tracker)];
        for turn in input["turns"].as_array().unwrap() {
            let items: Vec<_> = turn
                .as_array()
                .unwrap()
                .iter()
                .map(|event| match event["kind"].as_str().unwrap() {
                    "call" => RunItem::ToolCall {
                        call: ToolCall {
                            id: "call".into(),
                            name: event["name"].as_str().unwrap().into(),
                            arguments: serde_json::from_str(event["input"].as_str().unwrap())
                                .unwrap_or(Value::Null),
                            raw_arguments: Some(event["input"].as_str().unwrap().into()),
                        },
                    },
                    "error" => RunItem::ToolResult {
                        call_id: "call".into(),
                        output: ToolOutput {
                            content: vec![Content::Text {
                                text: event["text"].as_str().unwrap().into(),
                            }],
                            is_error: true,
                            should_pause: false,
                        },
                    },
                    _ => panic!("unknown oracle event"),
                })
                .collect();
            tracker.update(&items);
            states.push(state(&tracker));
        }
        assert_eq!(json!(states), case["states"], "{}", input["name"]);
    }
}
