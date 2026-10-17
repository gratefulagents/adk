#![cfg(feature = "host")]
use adk::{
    core::{Context, ErrorCategory, ToolCall},
    host::{GuardrailRule, compile_guardrail_rules},
    runtime::{CancellationToken, GuardrailInput},
};
use std::sync::Arc;

fn rule(pattern: &str) -> GuardrailRule {
    GuardrailRule {
        name: "commands".into(),
        regex: pattern.into(),
        action: "block".into(),
        rule_type: "tool-input".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn literal_shell_operators_are_not_character_class_set_syntax() {
    let context = Context {
        run_id: "rules".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    };
    for (pattern, command) in [
        ("git push.*--force", "git push origin main --force"),
        ("echo.*&&.*rm", "echo ready && rm file"),
        ("~~backup", "~~backup"),
        (r"[\&\&]", "&"),
        (r"\[a--b\]", "[a--b]"),
        (r"\Q[a&&b](?u)\E", "[a&&b](?u)"),
        (r"\Q(?x)\E", "(?x)"),
    ] {
        let compiled = compile_guardrail_rules(&[rule(pattern)]).unwrap();
        let call = ToolCall {
            raw_arguments: None,
            id: "call".into(),
            name: "shell".into(),
            arguments: serde_json::json!({"command": command}),
        };
        let result = compiled.input[0]
            .check(&context, "agent", GuardrailInput::ToolInput(&call))
            .await
            .unwrap()
            .unwrap();
        assert!(result.tripwire_triggered, "{pattern}");
    }
}

#[test]
fn quoting_does_not_enable_rust_only_flags_or_class_escapes() {
    for pattern in [r"\Qa\E(?u)b", r"(?x)\Qabc\E", r"(?R)a", r"(?-u)a"] {
        let error = compile_guardrail_rules(&[rule(pattern)]).err().unwrap();
        assert_eq!(error.info.category, ErrorCategory::Unsupported, "{pattern}");
    }
    for pattern in [r"[\Qa\E]", r"\\Qa\E", r"\E"] {
        assert!(
            compile_guardrail_rules(&[rule(pattern)]).is_err(),
            "{pattern}"
        );
    }
}

#[test]
fn unescaped_class_set_operators_remain_explicitly_unsupported() {
    for pattern in [
        "[a&&b]", "[a--b]", "[a~~b]", "[a||b]", "[]a&&b]", "[^]a&&b]",
    ] {
        let error = match compile_guardrail_rules(&[rule(pattern)]) {
            Ok(_) => panic!("unexpected set syntax accepted: {pattern}"),
            Err(error) => error,
        };
        assert_eq!(error.info.category, ErrorCategory::Unsupported, "{pattern}");
    }
}

#[test]
fn original_rule_compilation_rejects_unknown_type_and_action() {
    for (rule_type, action) in [("tool-inptu", "block"), ("tool-input", "deny")] {
        let mut rule = rule("x");
        rule.name = "r1".into();
        rule.rule_type = rule_type.into();
        rule.action = action.into();
        assert!(compile_guardrail_rules(&[rule]).is_err());
    }
}

#[tokio::test]
async fn original_empty_rule_action_defaults_to_block() {
    let mut rule = rule("danger");
    rule.name = "r1".into();
    rule.action.clear();
    let guards = compile_guardrail_rules(&[rule]).unwrap();
    assert_eq!(guards.input.len(), 1);
    let context = Context {
        run_id: "rules".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    };
    let call = ToolCall {
        id: "call".into(),
        name: "stub".into(),
        raw_arguments: None,
        arguments: serde_json::json!({"x":"danger"}),
    };
    let result = guards.input[0]
        .check(&context, "agent", GuardrailInput::ToolInput(&call))
        .await
        .unwrap()
        .unwrap();
    assert!(result.tripwire_triggered);
}

#[tokio::test]
async fn host_rule_action_normalization_uses_go_unicode_lowercase() {
    let context = Context {
        run_id: "unicode-action".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    };
    let mut input = rule("danger");
    input.action = " \tblocK\u{0085}".into();
    let compiled = compile_guardrail_rules(&[input]).unwrap();
    let call = ToolCall {
        id: "call".into(),
        name: "shell".into(),
        arguments: serde_json::json!({"x":"danger"}),
        raw_arguments: None,
    };
    let result = compiled.input[0]
        .check(&context, "agent", GuardrailInput::ToolInput(&call))
        .await
        .unwrap()
        .unwrap();
    assert!(result.tripwire_triggered);
}
