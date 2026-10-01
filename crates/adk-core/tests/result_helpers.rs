use adk_core::*;
use serde_json::json;

fn result() -> RunResult {
    RunResult {
        metrics: None,
        status: RunStatus::Completed,
        final_output: None,
        new_items: vec![],
        new_items_provenance: vec![],
        history: vec![],
        history_provenance: vec![],
        responses: vec![],
        usage: Usage::default(),
        pending_approvals: vec![],
        last_agent: None,
        guardrails: vec![],
    }
}

#[test]
fn final_text_does_not_serialize_structured_values() {
    let mut result = result();
    for value in [
        None,
        Some(json!(null)),
        Some(json!(false)),
        Some(json!(7)),
        Some(json!([])),
        Some(json!({"text":"not plain"})),
    ] {
        result.final_output = value;
        assert_eq!(result.final_text(), "");
    }
    result.final_output = Some(json!("  Unicode 🦀\n"));
    assert_eq!(result.final_text(), "  Unicode 🦀\n");
}

#[test]
fn final_text_matches_independent_go_result_fixtures() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/host-loop/sdk-conversation.json"
    ))
    .unwrap();
    assert_eq!(
        fixture["sdk_revision"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    let cases = fixture["result_helpers"].as_array().unwrap();
    let mut output_cases = 0;
    for case in cases {
        let mut result = result();
        result.final_output = case["input"].get("final_output").cloned();
        assert_eq!(
            result.final_text(),
            case["output"]["final_text"].as_str().unwrap(),
            "{}",
            case["name"]
        );
        if case["name"].as_str().unwrap().starts_with("final-output-") {
            output_cases += 1;
        }
    }
    assert_eq!(
        output_cases, 7,
        "all string/nonstring/missing output cases must run"
    );
}

#[test]
fn result_views_borrow_only_new_items_and_unified_approvals() {
    let mut result = result();
    result.status = RunStatus::Paused;
    result.history.push(RunItem::Message {
        message: Message {
            role: Role::User,
            content: vec![Content::Text {
                text: "earlier input".into(),
            }],
        },
    });
    assert!(
        !result.is_interrupted(),
        "tool pause is not approval interruption"
    );
    assert!(result.to_input_list().is_empty());
    for id in ["one", "two"] {
        result.pending_approvals.push(ApprovalRequest {
            call: ToolCall {
                id: id.into(),
                name: "review".into(),
                arguments: json!({}),
            },
            reason: "approval required".into(),
        });
    }
    result.new_items.push(RunItem::Message {
        message: Message {
            role: Role::Assistant,
            content: vec![Content::Text {
                text: "new output".into(),
            }],
        },
    });
    assert!(result.is_interrupted());
    assert_eq!(result.all_interruptions().len(), 2);
    assert_eq!(result.all_interruptions()[1].call.id, "two");
    assert!(std::ptr::eq(
        result.all_interruptions(),
        result.pending_approvals.as_slice()
    ));
    assert!(std::ptr::eq(
        result.to_input_list(),
        result.new_items.as_slice()
    ));
    assert_ne!(result.to_input_list(), result.history);
}
