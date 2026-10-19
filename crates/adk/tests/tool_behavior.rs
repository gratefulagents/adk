use adk::core::{RunPolicy, ToolUseBehavior};

#[test]
fn named_tool_behaviors_are_available_with_native_defaults_through_facade() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/tool-stopping/observations.json"
    ))
    .unwrap();
    assert_eq!(fixture["constants"], serde_json::json!([0, 1]));
    assert_eq!(ToolUseBehavior::default(), ToolUseBehavior::Continue);
    assert_eq!(RunPolicy::default().tool_use, ToolUseBehavior::Continue);
    let policy = RunPolicy {
        tool_use: ToolUseBehavior::StopAfterTool,
        ..Default::default()
    };
    assert_eq!(policy.tool_use, ToolUseBehavior::StopAfterTool);
}
