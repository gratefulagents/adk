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

fn tool_call(name: &str, input: &str) -> RunItem {
    RunItem::ToolCall {
        call: ToolCall {
            id: "call".into(),
            name: name.into(),
            arguments: serde_json::from_str(input).unwrap_or(Value::Null),
            raw_arguments: Some(input.into()),
        },
    }
}

#[test]
fn tracker_detects_no_tool_stall_at_original_threshold() {
    let mut tracker = AutoTracker::default();
    let thinking = RunItem::Message {
        message: adk_core::Message {
            role: adk_core::Role::Assistant,
            content: vec![Content::Text {
                text: "thinking".into(),
            }],
        },
    };
    for _ in 0..4 {
        tracker.update(std::slice::from_ref(&thinking));
        assert!(!tracker.check_circuit_breakers().tripped);
    }
    tracker.update(&[thinking]);
    let breaker = tracker.check_circuit_breakers();
    assert!(breaker.tripped);
    assert!(breaker.reason.contains("no tool calls"));
}

#[test]
fn smart_nudge_warns_on_original_repeated_no_tool_turns() {
    let mut tracker = AutoTracker::default();
    for _ in 0..3 {
        tracker.update(&[RunItem::Message {
            message: adk_core::Message {
                role: adk_core::Role::Assistant,
                content: vec![Content::Text {
                    text: "done soon".into(),
                }],
            },
        }]);
    }
    let nudge = build_smart_nudge(Some(&tracker), "shipping");
    for expected in [
        "WARNING",
        "tool calls",
        "Finish is a hard stop",
        "not a progress report",
        "backlog item",
        "call a tool for the top item",
    ] {
        assert!(nudge.contains(expected), "{expected}: {nudge}");
    }
}

#[test]
fn tracker_detects_original_repeated_tool_cycle() {
    let mut tracker = AutoTracker::default();
    for _ in 0..3 {
        tracker.update(&[tool_call("grep", ""), tool_call("bash", "")]);
    }
    let breaker = tracker.check_circuit_breakers();
    assert!(breaker.tripped);
    assert!(breaker.reason.contains("tool loop"));
}

#[test]
fn tracker_ignores_original_diverse_exploration() {
    let mut tracker = AutoTracker::default();
    for (name, input) in [
        ("grep", r#"{"pattern":"funcA"}"#),
        ("read_file", r#"{"path":"a.go"}"#),
        ("read_file", r#"{"path":"b.go"}"#),
        ("grep", r#"{"pattern":"funcB"}"#),
        ("read_file", r#"{"path":"c.go"}"#),
        ("grep", r#"{"pattern":"funcC"}"#),
        ("read_file", r#"{"path":"d.go"}"#),
        ("read_file", r#"{"path":"e.go"}"#),
        ("grep", r#"{"pattern":"funcD"}"#),
        ("read_file", r#"{"path":"f.go"}"#),
    ] {
        tracker.update(&[tool_call(name, input)]);
        assert!(!tracker.check_circuit_breakers().tripped);
    }
    assert!(!build_smart_nudge(Some(&tracker), "plan").contains("stuck in a loop"));
}

#[test]
fn tracker_detects_original_identical_call_loop() {
    let mut tracker = AutoTracker::default();
    for _ in 0..6 {
        tracker.update(&[tool_call("read_file", r#"{"path":"same.go"}"#)]);
    }
    let breaker = tracker.check_circuit_breakers();
    assert!(breaker.tripped);
    assert!(breaker.reason.contains("tool loop"));
}
