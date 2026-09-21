use std::{collections::BTreeSet, error::Error as _, sync::Arc, time::Instant};

use adk_core::*;
use serde_json::{Value, json};

struct NeverCancelled;

impl Cancellation for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}

#[test]
fn traits_are_object_safe_and_send_sync() {
    fn shared<T: ?Sized + Send + Sync>() {}
    fn send<T: ?Sized + Send>() {}
    shared::<dyn Agent>();
    shared::<dyn Model>();
    shared::<dyn StreamingModel>();
    shared::<dyn Tool>();
    shared::<dyn Host>();
    shared::<dyn Cancellation>();
    shared::<Context>();
    shared::<Error>();
    send::<dyn ModelStream>();
    send::<BoxFuture<'_, Result<RunResult, RunError>>>();
}

#[test]
fn context_checks_monotonic_deadline() {
    let mut context = Context {
        run_id: "run-1".into(),
        cancellation: Arc::new(NeverCancelled),
        deadline: None,
    };
    assert!(context.check_active().is_ok());
    context.deadline = Some(Instant::now());
    assert_eq!(
        context.check_active().unwrap_err().info.category,
        ErrorCategory::DeadlineExceeded
    );
}

#[test]
fn policy_is_exact_name_fail_closed_and_approval_is_additive() {
    let mut tool = ToolDefinition {
        name: "write_file".into(),
        description: "Write a file".into(),
        input_schema: schemars::schema_for!(Value),
        read_only: false,
        requires_approval: true,
    };
    let mut policy = ToolPolicy::default();
    assert_eq!(policy.decision(&tool), ToolDecision::Deny);
    policy.allowed_mutating_tools.insert("write".into());
    assert_eq!(policy.decision(&tool), ToolDecision::Deny);
    policy.allowed_mutating_tools.insert(tool.name.clone());
    assert_eq!(policy.decision(&tool), ToolDecision::RequireApproval);
    policy.allowed_tools = Some(BTreeSet::new());
    assert_eq!(policy.decision(&tool), ToolDecision::Deny);
    policy
        .allowed_tools
        .as_mut()
        .unwrap()
        .insert(tool.name.clone());
    assert_eq!(policy.decision(&tool), ToolDecision::RequireApproval);
    policy.denied_tools.insert(tool.name.clone());
    assert_eq!(policy.decision(&tool), ToolDecision::Deny);
    policy.denied_tools.clear();
    tool.requires_approval = false;
    assert_eq!(policy.decision(&tool), ToolDecision::Allow);
    policy.approval = ApprovalPolicy::All;
    assert_eq!(policy.decision(&tool), ToolDecision::RequireApproval);
    tool.read_only = true;
    assert_eq!(ToolPolicy::default().decision(&tool), ToolDecision::Allow);
}

#[test]
fn partial_run_retains_history_usage_and_cause() {
    let item = RunItem::Message {
        message: Message {
            role: Role::Assistant,
            content: vec![Content::Text {
                text: "partial".into(),
            }],
        },
    };
    let partial = RunResult {
        status: RunStatus::Incomplete,
        final_output: None,
        new_items: vec![item.clone()],
        history: vec![item],
        responses: vec![],
        usage: Usage {
            output_tokens: 42,
            ..Usage::default()
        },
        pending_approvals: vec![],
        last_agent: Some("assistant".into()),
        guardrails: vec![],
    };
    let error = RunError::with_partial(
        Error::new(ErrorCategory::MaxTurns, "turn budget exhausted")
            .with_source(std::io::Error::other("original cause")),
        partial.clone(),
    );
    assert_eq!(error.partial.as_deref(), Some(&partial));
    assert_eq!(error.to_string(), "turn budget exhausted");
    assert_eq!(
        error.source().unwrap().source().unwrap().to_string(),
        "original cause"
    );
    let encoded = serde_json::to_value(&partial).unwrap();
    assert_eq!(
        serde_json::from_value::<RunResult>(encoded).unwrap(),
        partial
    );
}

#[test]
fn native_items_preserve_order_ids_and_arbitrary_precision() {
    let number = "1234567890123456789012345678901234567890";
    let items = vec![
        RunItem::ToolCall {
            call: ToolCall {
                id: "call-7".into(),
                name: "calculate".into(),
                arguments: serde_json::from_str(number).unwrap(),
            },
        },
        RunItem::ToolResult {
            call_id: "call-7".into(),
            output: ToolOutput {
                content: vec![Content::Text {
                    text: "done".into(),
                }],
                is_error: false,
                should_pause: false,
            },
        },
    ];
    let encoded = serde_json::to_string(&items).unwrap();
    assert!(encoded.contains(number));
    assert_eq!(
        serde_json::from_str::<Vec<RunItem>>(&encoded).unwrap(),
        items
    );
    let unknown = json!({"type": "future_unknown_variant"});
    assert!(serde_json::from_value::<RunItem>(unknown).is_err());
}

#[test]
fn model_end_turn_preserves_absent_false_and_true() {
    for end_turn in [None, Some(false), Some(true)] {
        let response = ModelResponse {
            items: vec![],
            usage: Usage::default(),
            end_turn,
            response_id: None,
            metadata: Default::default(),
        };
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(
            serde_json::from_value::<ModelResponse>(encoded).unwrap(),
            response
        );
    }
}

#[test]
fn schemas_expose_tagged_events_and_nonzero_turn_limit() {
    let schema = serde_json::to_value(schemars::schema_for!(RunEvent)).unwrap();
    assert_eq!(schema["oneOf"].as_array().unwrap().len(), 7);
    assert!(
        schema["$defs"]["ToolCall"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("id"))
    );
    let schema = serde_json::to_value(schemars::schema_for!(RunPolicy)).unwrap();
    assert_eq!(schema["properties"]["max_turns"]["minimum"], 1);
    let invalid = json!({"max_turns": 0, "tools": ToolPolicy::default(), "tool_use": "continue"});
    assert!(serde_json::from_value::<RunPolicy>(invalid).is_err());
}

#[test]
fn default_run_policy_preserves_baseline_hundred_turn_budget() {
    let policy = RunPolicy::default();
    assert_eq!(policy.max_turns.get(), 100);
    assert_eq!(policy.tools, ToolPolicy::default());
    assert_eq!(policy.tool_use, ToolUseBehavior::Continue);
}
