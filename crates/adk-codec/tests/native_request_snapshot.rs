use adk_codec::snapshots::{RequestSnapshot, to_go_json};
use adk_core::*;
use serde_json::{Value, json};
use std::time::Duration;

fn empty() -> ModelRequest {
    ModelRequest {
        model: "fixture/model".into(),
        instructions: String::new(),
        input: vec![],
        input_provenance: vec![],
        tools: vec![],
        output_schema: None,
        output_schema_name: String::new(),
        output_schema_strict: false,
        settings: Default::default(),
    }
}
fn message(role: Role, text: &str) -> Message {
    Message {
        role,
        content: vec![Content::Text { text: text.into() }],
    }
}
fn attributed() -> ItemProvenance {
    ItemProvenance::Agent { name: "A".into() }
}
fn full() -> ModelRequest {
    ModelRequest {
        instructions: "policy <>&\u{2028}\u{2029}".into(),
        input: vec![
            RunItem::Message { message: message(Role::User, "user") },
            RunItem::PhasedMessage { message: message(Role::Assistant, "answer <>&"), phase: "commentary".into() },
            RunItem::ToolCall { call: ToolCall { id: "call".into(), name: "tool".into(), arguments: json!({"x":"<>&"}) } },
            RunItem::ToolResult { call_id: "call".into(), output: ToolOutput { content: vec![Content::Text { text: "done".into() }], is_error: false, should_pause: false } },
            RunItem::Reasoning { reasoning: Reasoning { id: "reason".into(), text: "reasoning".into(), signature: "signature".into(), ..Default::default() } },
        ],
        input_provenance: vec![ItemProvenance::Unattributed, attributed(), attributed(), attributed(), attributed()],
        tools: vec![ToolDefinition { name: "tool".into(), description: "description".into(), input_schema: serde_json::from_value(json!({"properties":{"x":{"type":"string"}},"type":"object"})).unwrap(), read_only: true, requires_approval: true }],
        settings: json!({"temperature":0.5,"max_tokens":100,"parallel_tool_calls":false,"thinking_budget":200,"stop_sequences":["stop"],"prompt_cache_key":"private-cache-key"}).as_object().unwrap().clone(),
        output_schema: Some(serde_json::from_value(json!({"type":"object"})).unwrap()), output_schema_name: "answer".into(), output_schema_strict: true,
        ..empty()
    }
}
#[test]
fn native_requests_match_pinned_builder_bytes_and_token_estimates() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    let mut compact = empty();
    compact.model.clear();
    compact.input.push(RunItem::Compaction {
        compaction: Compaction {
            encrypted_content: "x".repeat(80004),
            ..Default::default()
        },
    });
    compact.input_provenance.push(attributed());
    let mut negative = empty();
    negative.model.clear();
    negative.settings = json!({"max_tokens":-1,"thinking_budget":20000})
        .as_object()
        .unwrap()
        .clone();
    for (name, request, timeouts) in [
        ("empty", empty(), vec![]),
        ("full", full(), vec![Some(Duration::from_secs(30))]),
        ("compaction_cap", compact, vec![]),
        ("negative_max", negative, vec![]),
    ] {
        let snapshot = RequestSnapshot::from_native("B", &request, &timeouts).unwrap();
        let bytes = to_go_json(&snapshot).unwrap();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            fixture["native_request_cases"][name].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            snapshot.total_token_estimate,
            snapshot.input_token_estimate + snapshot.request_overhead_token_estimate
        );
        if name == "full" {
            assert!(snapshot.input_items[0].agent_name.is_empty());
            assert_eq!(snapshot.input_items[1].agent_name, "A");
            assert_eq!(snapshot.agent_name, "B");
            assert!(
                !String::from_utf8(bytes)
                    .unwrap()
                    .contains("private-cache-key")
            );
        }
    }
}
#[test]
fn unknown_or_malformed_provenance_is_not_inferred_from_current_agent() {
    for provenance in [
        vec![],
        vec![ItemProvenance::Unknown],
        vec![ItemProvenance::Agent { name: " ".into() }],
        vec![attributed(), attributed()],
    ] {
        let mut request = empty();
        request.input.push(RunItem::Message {
            message: message(Role::Assistant, "old answer"),
        });
        request.input_provenance = provenance;
        assert!(RequestSnapshot::from_native("current", &request, &[]).is_err());
    }
    let mut request = empty();
    request.input.push(RunItem::Message {
        message: message(Role::Assistant, "explicitly unattributed"),
    });
    request.input_provenance.push(ItemProvenance::Unattributed);
    assert!(
        RequestSnapshot::from_native("current", &request, &[])
            .unwrap()
            .input_items[0]
            .agent_name
            .is_empty()
    );
}
#[test]
fn unsupported_settings_timeouts_and_items_fail_without_silent_loss() {
    let request = full();
    assert!(RequestSnapshot::from_native("B", &request, &[]).is_err());
    for timeout in [Duration::from_millis(1), Duration::from_secs(u64::MAX)] {
        assert!(RequestSnapshot::from_native("B", &request, &[Some(timeout)]).is_err());
    }
    let zero = RequestSnapshot::from_native("B", &request, &[None]).unwrap();
    assert_eq!(zero.tools[0].timeout_seconds, 0);
    for (key, value) in [
        ("adapter_extra", json!(true)),
        ("max_tokens", json!(u64::MAX)),
        ("max_tokens", json!(1.5)),
        ("tool_choice", json!({"name":"tool"})),
        ("prompt_cache_key", json!(7)),
    ] {
        let mut invalid = request.clone();
        invalid.settings.insert(key.into(), value);
        assert!(
            RequestSnapshot::from_native("B", &invalid, &[None]).is_err(),
            "{key}"
        );
    }
    let mut invalid = empty();
    invalid.input.push(RunItem::Message {
        message: Message {
            role: Role::User,
            content: vec![Content::File {
                uri: "file:///private".into(),
                media_type: "text/plain".into(),
            }],
        },
    });
    invalid.input_provenance.push(ItemProvenance::Unattributed);
    assert!(RequestSnapshot::from_native("B", &invalid, &[]).is_err());
}

#[test]
fn approval_markers_keep_order_authorship_and_estimates() {
    use adk_codec::approval::{ApprovalMarker, ApprovalMarkerBoundary, ApprovalPhase};
    use adk_codec::dto::{AgentRef, SnapshotType};
    let request = full();
    let marker = ApprovalMarker::from_call(
        &ToolCall {
            id: "call".into(),
            name: "tool".into(),
            arguments: json!({"x":"<>&"}),
        },
        ApprovalPhase::Approved,
        Some(AgentRef { name: "A".into() }),
    );
    let boundary = ApprovalMarkerBoundary {
        before_item: 3,
        marker,
    };
    let snapshot = RequestSnapshot::from_native_with_approvals(
        "B",
        &request,
        &[Some(Duration::from_secs(30))],
        &[boundary.clone()],
    )
    .unwrap();
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/tracestore/sdk-writer.json")).unwrap();
    assert_eq!(
        String::from_utf8(to_go_json(&snapshot).unwrap()).unwrap(),
        fixture["native_request_cases"]["approved"]
            .as_str()
            .unwrap()
    );
    assert_eq!(snapshot.input_items[3].kind, SnapshotType::ToolApproval);
    assert_eq!(snapshot.input_items[3].agent_name, "A");
    let mut denied = boundary.clone();
    denied.marker.phase = ApprovalPhase::Denied;
    denied.marker.data.approved = false;
    denied.marker.agent = None;
    let both = RequestSnapshot::from_native_with_approvals(
        "B",
        &request,
        &[None],
        &[boundary.clone(), denied.clone()],
    )
    .unwrap();
    assert!(both.input_items[3].tool_approval.as_ref().unwrap().approved);
    assert!(!both.input_items[4].tool_approval.as_ref().unwrap().approved);
    assert!(both.input_items[4].agent_name.is_empty());
    denied.before_item = 2;
    assert!(
        RequestSnapshot::from_native_with_approvals(
            "B",
            &request,
            &[None],
            &[boundary.clone(), denied]
        )
        .is_err()
    );
    let outside = ApprovalMarkerBoundary {
        before_item: 100,
        ..boundary
    };
    assert!(
        RequestSnapshot::from_native_with_approvals("B", &request, &[None], &[outside]).is_err()
    );
}

#[test]
fn overflowing_float_settings_fail_at_construction() {
    for field in ["temperature", "top_p"] {
        for number in ["1e309", "-1e309"] {
            let mut request = full();
            request.settings = serde_json::from_str(&format!("{{\"{field}\":{number}}}")).unwrap();
            assert!(RequestSnapshot::from_native("B", &request, &[None]).is_err());
        }
    }
}
