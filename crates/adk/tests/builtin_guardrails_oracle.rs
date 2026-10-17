#![cfg(feature = "builder")]
//! Bounded outcomes from independently executed SDK guards, not a duplicated matcher.
use adk::{core::*, guardrails::*, runtime::*};
use serde_json::{Value, json};
use std::sync::Arc;

fn recipe(case: &Value) -> String {
    let recipe = &case["recipe"];
    assert_eq!(recipe["version"], 1);
    let kind = recipe["kind"].as_str().unwrap();
    if kind == "literal" {
        return case[if case["phase"] == "input" {
            "params"
        } else {
            "content"
        }]
        .as_str()
        .unwrap()
        .into();
    }
    assert_eq!(recipe["length"], 36);
    let github = format!("ghp{}{}", "_", "a".repeat(36));
    let aws = format!(
        "{}IA{}",
        if kind == "aws_temporary_pair" {
            "AS"
        } else {
            "AK"
        },
        "A".repeat(16)
    );
    let companion = "z".repeat(36);
    let begin = ["-----BEGIN RSA PRIVATE ", "KEY-----"].concat();
    let end = ["-----END RSA PRIVATE ", "KEY-----"].concat();
    let body = "Q".repeat(36);
    let payload = match kind {
        "github_token" => format!("token={github}"),
        "github_repeated" => format!("token={github}\nagain={github}"),
        "pem_closed" => format!("{begin}\n{body}\n{end}"),
        "pem_unterminated" => format!("{begin}\n{body}"),
        "github_and_pem" => format!("token={github}\n{begin}\n{body}\n{end}"),
        "aws_pair" | "aws_temporary_pair" => {
            format!("AWS_ACCESS_KEY_ID={aws}\nAWS_SECRET_ACCESS_KEY={companion}")
        }
        "github_and_aws" => {
            format!("token={github}\nAWS_ACCESS_KEY_ID={aws}\nAWS_SECRET_ACCESS_KEY={companion}")
        }
        "gcp_service_account" => format!(
            "{{\"type\": \"{}\", \"private_key_id\": \"{companion}\"}}",
            ["service", "_account"].concat()
        ),
        _ => panic!("unknown synthetic recipe"),
    };
    let payload = if kind == "gcp_service_account" {
        payload
    } else {
        format!("before\n{payload}\nafter")
    };
    if recipe["encoding"] == "json_text" {
        json!({"text":payload}).to_string()
    } else {
        payload
    }
}

#[tokio::test]
async fn builtin_dispositions_and_redacted_bytes_match_pinned_sdk_where_representable() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-builtin-guardrails.json"
    ))
    .unwrap();
    assert_eq!(fixture["schema_version"], 1);
    assert_eq!(
        fixture["sdk_revision"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    let inputs = builtin_tool_input_guardrails();
    let outputs = builtin_tool_output_guardrails();
    assert_eq!(
        json!(inputs.iter().map(|g| g.name()).collect::<Vec<_>>()),
        fixture["input_guard_names"]
    );
    assert_eq!(
        json!(outputs.iter().map(|g| g.name()).collect::<Vec<_>>()),
        fixture["output_guard_names"]
    );
    let context = Context {
        run_id: "builtin-oracle".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    };
    let mut matched = 0;
    let mut hardened = 0;
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 97);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let source = recipe(case);
        let input = case["phase"] == "input";
        let arguments = if input {
            serde_json::from_str(&source).unwrap_or(Value::Null)
        } else {
            json!({})
        };
        let call = ToolCall {
            raw_arguments: input.then(|| source.clone()),
            id: "call".into(),
            name: case["tool_name"].as_str().unwrap().into(),
            arguments,
        };
        let mut output = ToolOutput {
            content: vec![Content::Text { text: source }],
            is_error: false,
            should_pause: false,
        };
        let guards = if input { &inputs } else { &outputs };
        let mut actual = vec![];
        let mut blocked = false;
        for guard in guards {
            let checked = guard
                .check(
                    &context,
                    "oracle",
                    if input {
                        GuardrailInput::ToolInput(&call)
                    } else {
                        GuardrailInput::ToolOutput {
                            call: &call,
                            output: &output,
                        }
                    },
                )
                .await
                .unwrap()
                .unwrap_or_default();
            actual.push((
                guard.name(),
                checked.tripwire_triggered,
                checked.replacement_content.is_some(),
            ));
            if checked.tripwire_triggered {
                blocked = true;
                break;
            }
            if let Some(text) = checked.replacement_content {
                output.content = vec![Content::Text { text }];
            }
        }
        let expected: Vec<_> = case["guards"]
            .as_array()
            .unwrap()
            .iter()
            .map(|guard| {
                assert_eq!(guard["error"], "", "{name}");
                (
                    guard["name"].as_str().unwrap(),
                    guard["result"]["TripwireTriggered"].as_bool().unwrap(),
                    guard["result"]["ContentReplaced"].as_bool().unwrap(),
                )
            })
            .collect();
        if name == "input_gcp_escaped_json_text" {
            // Native scans decoded JSON strings as well: intentional stronger secret protection.
            assert!(expected.iter().all(|(_, tripped, _)| !tripped));
            assert_eq!(
                actual,
                vec![
                    ("block-destructive-commands", false, false),
                    ("detect-secret-leak", true, false)
                ]
            );
            hardened += 1;
            continue;
        }
        assert_eq!(actual, expected, "disposition mismatch in {name}");
        if !input {
            let final_content = if blocked {
                Value::Null
            } else {
                let Content::Text { text } = &output.content[0] else {
                    panic!("unexpected output type")
                };
                json!(text)
            };
            assert_eq!(
                final_content, case["final_content"],
                "redacted-byte mismatch in {name}"
            );
        }
        matched += 1;
    }
    assert_eq!((matched, hardened), (96, 1));
}

#[test]
fn public_tool_pattern_helper_matches_pinned_sdk_not_chatloop_globs() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-builtin-guardrails.json"
    ))
    .unwrap();
    let cases = fixture["tool_patterns"].as_array().unwrap();
    assert_eq!(cases.len(), 120);
    for case in cases {
        assert_eq!(
            adk::guardrails::match_tool_pattern(
                case["name"].as_str().unwrap(),
                case["pattern"].as_str().unwrap()
            ),
            case["matches"].as_bool().unwrap(),
            "{case}"
        );
    }
}

#[cfg(feature = "host")]
#[tokio::test]
async fn public_rule_callbacks_match_pinned_sdk_observations() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-builtin-guardrails.json"
    ))
    .unwrap();
    let cases = fixture["public_rules"].as_array().unwrap();
    assert_eq!(cases.len(), 124);
    let context = Context {
        run_id: "public-rule-oracle".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    };
    for case in cases {
        let source = &case["rule"];
        let rule = public_rule(source);
        let regex = regex::Regex::new(case["regex"].as_str().unwrap()).unwrap();
        let is_input = case["phase"] == "input";
        let guard = if is_input {
            make_tool_input_guardrail(rule, regex)
        } else {
            make_tool_output_guardrail(rule, regex)
        };
        assert_eq!(
            guard.name(),
            format!("crd:{}", source["Name"].as_str().unwrap())
        );
        let text = case["text"].as_str().unwrap();
        let call = ToolCall {
            id: "call".into(),
            name: case["tool"].as_str().unwrap().into(),
            arguments: Value::Null,
            raw_arguments: Some(text.into()),
        };
        let output = ToolOutput {
            content: vec![Content::Text { text: text.into() }],
            is_error: false,
            should_pause: false,
        };
        let input = if is_input {
            GuardrailInput::ToolInput(&call)
        } else {
            GuardrailInput::ToolOutput {
                call: &call,
                output: &output,
            }
        };
        let result = guard
            .check(&context, "agent", input)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(case["error"], "");
        assert_eq!(result.output, case["result"]["Output"], "{case}");
        assert_eq!(
            result.tripwire_triggered,
            case["result"]["TripwireTriggered"].as_bool().unwrap(),
            "{case}"
        );
        assert_eq!(result.replacement_content, None);
        assert_eq!(case["result"]["ContentReplaced"], false);
        let wrong_phase = if is_input {
            GuardrailInput::ToolOutput {
                call: &call,
                output: &output,
            }
        } else {
            GuardrailInput::ToolInput(&call)
        };
        assert!(
            guard
                .check(&context, "agent", wrong_phase)
                .await
                .unwrap()
                .is_none()
        );
        assert!(guard.durable_key().is_none());
    }
}

#[cfg(feature = "host")]
fn public_rule(source: &Value) -> adk::host::GuardrailRule {
    adk::host::GuardrailRule {
        name: source["Name"].as_str().unwrap().into(),
        regex: source["Regex"].as_str().unwrap().into(),
        action: source["Action"].as_str().unwrap().into(),
        rule_type: source["Type"].as_str().unwrap().into(),
        tool_pattern: source["ToolPattern"].as_str().unwrap().into(),
        message: source["Message"].as_str().unwrap().into(),
    }
}

#[cfg(feature = "host")]
#[tokio::test]
async fn public_rule_compiler_retains_valid_rules_and_ordered_errors() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-builtin-guardrails.json"
    ))
    .unwrap();
    let cases = fixture["compiled_rules"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    let context = Context {
        run_id: "compiled-rule-oracle".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    };
    let call = ToolCall {
        id: "call".into(),
        name: "reader".into(),
        arguments: Value::Null,
        raw_arguments: Some("danger [a&&b]".into()),
    };
    let output = ToolOutput {
        content: vec![Content::Text {
            text: "danger [a&&b]".into(),
        }],
        is_error: false,
        should_pause: false,
    };
    for case in cases {
        let rules: Vec<_> = case["rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(public_rule)
            .collect();
        let (compiled, errors) = tool_guardrails_from_rules(&rules);
        let expected_errors = case["errors"].as_array().map_or(&[][..], Vec::as_slice);
        assert_eq!(errors.len(), expected_errors.len(), "{case}");
        for (actual, expected) in errors.iter().zip(expected_errors) {
            assert_eq!(actual.info.category, ErrorCategory::InvalidInput);
            let expected = expected.as_str().unwrap();
            if expected.starts_with("invalid regex") {
                // Native parser prose is deliberately not claimed as RE2 parity.
                assert!(actual.info.message.contains("invalid-regex"));
                assert!(
                    actual
                        .info
                        .message
                        .starts_with("invalid or unsupported Go/Rust regex")
                );
            } else {
                assert_eq!(actual.info.message, expected);
            }
        }
        if case["name"] == "mixed" {
            assert!(adk::host::compile_guardrail_rules(&rules).is_err());
        }
        for (phase, guards) in [("input", compiled.input), ("output", compiled.output)] {
            let expected = case[phase].as_array().map_or(&[][..], Vec::as_slice);
            assert_eq!(guards.len(), expected.len(), "{case}");
            for (guard, expected) in guards.iter().zip(expected) {
                assert_eq!(guard.name(), expected["name"].as_str().unwrap());
                assert_eq!(expected["error"], "");
                let input = if phase == "input" {
                    GuardrailInput::ToolInput(&call)
                } else {
                    GuardrailInput::ToolOutput {
                        call: &call,
                        output: &output,
                    }
                };
                let actual = guard
                    .check(&context, "agent", input)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(actual.output, expected["result"]["Output"], "{case}");
                assert_eq!(
                    actual.tripwire_triggered,
                    expected["result"]["TripwireTriggered"].as_bool().unwrap(),
                    "{case}"
                );
                assert!(actual.replacement_content.is_none());
                assert_eq!(expected["result"]["ContentReplaced"], false);
            }
        }
    }
}

#[cfg(feature = "host")]
#[test]
fn public_rule_compiler_preserves_explicit_regex_support_limits() {
    let rules: Vec<_> = ["(?x)danger", "[a--b]", "danger"]
        .into_iter()
        .map(|pattern| adk::host::GuardrailRule {
            name: pattern.into(),
            regex: pattern.into(),
            rule_type: "tool-input".into(),
            ..Default::default()
        })
        .collect();
    let (compiled, errors) = tool_guardrails_from_rules(&rules);
    assert_eq!(compiled.input.len(), 1);
    assert!(compiled.output.is_empty());
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].info.category, ErrorCategory::Unsupported);
    assert_eq!(errors[1].info.category, ErrorCategory::InvalidInput);
}

#[cfg(feature = "host")]
#[tokio::test]
async fn posix_guardrail_classes_match_actual_go_membership() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-builtin-guardrails.json"
    ))
    .unwrap();
    let cases = fixture["regex_classes"].as_array().unwrap();
    let inputs = fixture["regex_inputs"].as_array().unwrap();
    assert_eq!(cases.len(), 62);
    assert_eq!(inputs.len(), 142);
    assert_regex_membership(cases, inputs).await;
}

#[cfg(feature = "host")]
#[tokio::test]
async fn literal_count_syntax_matches_actual_go_membership() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-builtin-guardrails.json"
    ))
    .unwrap();
    let cases = fixture["regex_literals"].as_array().unwrap();
    let inputs = fixture["regex_literal_inputs"].as_array().unwrap();
    assert_eq!(cases.len(), 23);
    assert_eq!(inputs.len(), 23);
    assert_regex_membership(cases, inputs).await;
}

#[cfg(feature = "host")]
async fn assert_regex_membership(cases: &[Value], inputs: &[Value]) {
    let context = Context {
        run_id: "class-oracle".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    };
    for case in cases {
        let pattern = case["pattern"].as_str().unwrap();
        let rule = adk::host::GuardrailRule {
            name: "class".into(),
            regex: pattern.into(),
            rule_type: "tool-input".into(),
            ..Default::default()
        };
        let host = adk::host::compile_guardrail_rules(std::slice::from_ref(&rule));
        let (public, errors) = tool_guardrails_from_rules(&[rule]);
        if case["error"] != "" {
            assert!(host.is_err(), "host accepted {pattern}");
            assert_eq!(errors.len(), 1, "public accepted {pattern}");
            assert!(public.input.is_empty());
            continue;
        }
        let host = host.unwrap();
        assert!(errors.is_empty(), "{pattern}: {errors:?}");
        let matches = case["matches"].as_str().unwrap();
        assert_eq!(matches.len(), inputs.len());
        for (expected, input) in matches.bytes().zip(inputs) {
            assert!(matches!(expected, b'0' | b'1'));
            let text = input.as_str().unwrap();
            let call = ToolCall {
                id: "call".into(),
                name: "reader".into(),
                arguments: Value::Null,
                raw_arguments: Some(text.into()),
            };
            for guard in host.input.iter().chain(&public.input) {
                let actual = guard
                    .check(&context, "agent", GuardrailInput::ToolInput(&call))
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    actual.tripwire_triggered,
                    expected == b'1',
                    "{pattern} / {text:?} / {}",
                    guard.name()
                );
            }
        }
    }
}

#[cfg(feature = "host")]
#[test]
fn guardrail_repeat_limits_match_actual_go_compilation() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-builtin-guardrails.json"
    ))
    .unwrap();
    let cases = fixture["regex_repeats"].as_array().unwrap();
    assert_eq!(cases.len(), 40);
    for case in cases {
        let pattern = case["pattern"].as_str().unwrap();
        let rule = adk::host::GuardrailRule {
            name: "repeat".into(),
            regex: pattern.into(),
            rule_type: "tool-input".into(),
            ..Default::default()
        };
        let accepted = case["error"] == "";
        let host = adk::host::compile_guardrail_rules(std::slice::from_ref(&rule));
        assert_eq!(host.is_ok(), accepted, "host: {pattern}");
        let (public, errors) = tool_guardrails_from_rules(&[rule]);
        assert_eq!(errors.is_empty(), accepted, "public: {pattern}");
        assert_eq!(public.input.len(), usize::from(accepted));
    }
}

#[cfg(feature = "host")]
#[tokio::test]
async fn unicode_property_names_match_actual_go_compilation() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-builtin-guardrails.json"
    ))
    .unwrap();
    let cases = fixture["regex_unicode_names"].as_array().unwrap();
    assert_eq!(cases.len(), 568);
    let names: Vec<_> = fixture["unicode_class_names"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 200);
    assert_eq!(
        include_str!("../src/host/go_unicode_names.txt")
            .lines()
            .collect::<Vec<_>>(),
        names
    );
    for case in cases {
        let pattern = case["pattern"].as_str().unwrap();
        let rule = adk::host::GuardrailRule {
            name: "unicode-name".into(),
            regex: pattern.into(),
            rule_type: "tool-input".into(),
            ..Default::default()
        };
        let accepted = case["error"] == "";
        let host = adk::host::compile_guardrail_rules(std::slice::from_ref(&rule));
        assert_eq!(host.is_ok(), accepted, "host: {pattern}");
        let (public, errors) = tool_guardrails_from_rules(&[rule]);
        assert_eq!(errors.is_empty(), accepted, "public: {pattern}");
        assert_eq!(public.input.len(), usize::from(accepted));
    }
}

#[cfg(feature = "host")]
#[tokio::test]
async fn unicode_property_negation_matches_actual_go_membership() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-builtin-guardrails.json"
    ))
    .unwrap();
    let cases = fixture["regex_unicode_membership"].as_array().unwrap();
    let inputs = fixture["regex_inputs"].as_array().unwrap();
    assert_eq!(cases.len(), 84);
    assert_eq!(inputs.len(), 142);
    assert_regex_membership(cases, inputs).await;
}

#[cfg(feature = "host")]
#[tokio::test]
async fn perl_class_atoms_match_actual_go_membership() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-builtin-guardrails.json"
    ))
    .unwrap();
    let cases = fixture["regex_perl_classes"].as_array().unwrap();
    let inputs = fixture["regex_inputs"].as_array().unwrap();
    assert_eq!(cases.len(), 40);
    assert_eq!(inputs.len(), 142);
    assert_regex_membership(cases, inputs).await;
}

#[cfg(feature = "host")]
#[tokio::test]
async fn class_set_symbols_remain_go_literals() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-builtin-guardrails.json"
    ))
    .unwrap();
    let cases = fixture["regex_set_literals"].as_array().unwrap();
    let inputs = fixture["regex_inputs"].as_array().unwrap();
    assert_eq!(cases.len(), 21);
    assert_eq!(inputs.len(), 142);
    assert_regex_membership(cases, inputs).await;
}

#[cfg(feature = "host")]
#[tokio::test]
async fn octal_escapes_match_actual_go_membership() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-builtin-guardrails.json"
    ))
    .unwrap();
    let cases = fixture["regex_octal"].as_array().unwrap();
    let inputs = fixture["regex_octal_inputs"].as_array().unwrap();
    assert_eq!(cases.len(), 45);
    assert_eq!(inputs.len(), 141);
    assert_regex_membership(cases, inputs).await;
}

#[cfg(feature = "host")]
#[tokio::test]
async fn capture_groups_match_actual_go_membership() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-builtin-guardrails.json"
    ))
    .unwrap();
    let cases = fixture["regex_captures"].as_array().unwrap();
    let inputs = fixture["regex_capture_inputs"].as_array().unwrap();
    assert_eq!(cases.len(), 22);
    assert_eq!(inputs.len(), 12);
    assert_regex_membership(cases, inputs).await;
}

#[cfg(feature = "host")]
#[tokio::test]
async fn inner_brackets_match_actual_go_literal_membership() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-builtin-guardrails.json"
    ))
    .unwrap();
    let cases = fixture["regex_brackets"].as_array().unwrap();
    let inputs = fixture["regex_bracket_inputs"].as_array().unwrap();
    assert_eq!(cases.len(), 21);
    assert_eq!(inputs.len(), 140);
    assert_regex_membership(cases, inputs).await;
}

#[cfg(feature = "host")]
#[tokio::test]
async fn hyphens_match_actual_go_range_membership() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-builtin-guardrails.json"
    ))
    .unwrap();
    let cases = fixture["regex_hyphens"].as_array().unwrap();
    let inputs = fixture["regex_hyphen_inputs"].as_array().unwrap();
    assert_eq!(cases.len(), 490);
    assert_eq!(inputs.len(), 261);
    assert_regex_membership(cases, inputs).await;
}
