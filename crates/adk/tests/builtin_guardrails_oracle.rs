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
    let mut unrepresentable = vec![];
    let mut matched = 0;
    let mut hardened = 0;
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 87);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let source = recipe(case);
        let input = case["phase"] == "input";
        let arguments = if input {
            match serde_json::from_str(&source) {
                Ok(arguments) => arguments,
                Err(_) => {
                    // Native ToolCall has a typed Value; raw malformed JSON cannot enter a guard.
                    unrepresentable.push(name);
                    continue;
                }
            }
        } else {
            json!({})
        };
        let call = ToolCall {
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
    unrepresentable.sort();
    assert_eq!(
        unrepresentable,
        vec![
            "name_BASH_malformed",
            "name_ExEcUtE_malformed",
            "name_ShElL_malformed",
            "name_bash_malformed",
            "name_custom_BaSh_tool_malformed",
            "name_exec_malformed",
            "name_read_file_malformed",
            "name_sh_malformed",
            "name_shell_malformed",
            "params_empty",
            "params_trailing",
        ]
    );
    assert_eq!((matched, hardened), (75, 1));
}
