use adk_core::*;
use adk_runtime::*;
use serde_json::json;
use std::sync::{Arc, Mutex};

#[test]
fn tripwire_labels_match_pinned_go_messages_and_every_unicode_scalar() {
    use sha2::{Digest, Sha256};
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/handoff/sdk-tripwire-errors.json"
    ))
    .unwrap();
    assert_eq!(fixture["unicode_version"], "15.0.0");
    assert_eq!(unicode_general_category::UNICODE_VERSION, (15, 0, 0));
    for case in fixture["cases"].as_array().unwrap() {
        let phase = match case["phase"].as_str().unwrap() {
            "input" => GuardrailPhase::Input,
            "output" => GuardrailPhase::Output,
            "tool input" => GuardrailPhase::ToolInput,
            "tool output" => GuardrailPhase::ToolOutput,
            other => panic!("unexpected phase {other}"),
        };
        let error = GuardrailTripwire {
            phase,
            name: case["name"].as_str().unwrap().into(),
            tool_name: Some(case["tool"].as_str().unwrap().into()),
            output: serde_json::Value::Null,
        };
        assert_eq!(error.to_string(), case["message"].as_str().unwrap());
    }
    let mut hash = Sha256::new();
    let mut count = 0;
    for ch in (0..=0x10ffff).filter_map(char::from_u32) {
        let label = format!("a{ch}b");
        for error in [
            GuardrailTripwire {
                phase: GuardrailPhase::Input,
                name: label.clone(),
                tool_name: None,
                output: serde_json::Value::Null,
            },
            GuardrailTripwire {
                phase: GuardrailPhase::ToolOutput,
                name: "guard".into(),
                tool_name: Some(label),
                output: serde_json::Value::Null,
            },
        ] {
            let message = error.to_string();
            hash.update((message.len() as u32).to_le_bytes());
            hash.update(message.as_bytes());
        }
        count += 1;
    }
    assert_eq!(fixture["scalar_count"], count);
    assert_eq!(fixture["scalar_sha256"], format!("{:x}", hash.finalize()));
}

#[test]
fn tripwire_messages_identify_phase_without_serializing_diagnostic_data() {
    for (phase, expected) in [
        (
            GuardrailPhase::Input,
            "input guardrail \"check\" tripwire triggered",
        ),
        (
            GuardrailPhase::Output,
            "output guardrail \"check\" tripwire triggered",
        ),
        (
            GuardrailPhase::ToolInput,
            "tool input guardrail \"check\" tripwire triggered for tool \"mutate\"",
        ),
        (
            GuardrailPhase::ToolOutput,
            "tool output guardrail \"check\" tripwire triggered for tool \"mutate\"",
        ),
    ] {
        let cause = GuardrailTripwire {
            phase,
            name: "check".into(),
            tool_name: Some("mutate".into()),
            output: json!({"private": "diagnostic-secret"}),
        };
        assert_eq!(cause.to_string(), expected);
        let error = Error::new(ErrorCategory::Guardrail, cause.to_string()).with_source(cause);
        assert!(
            !serde_json::to_string(&error.info)
                .unwrap()
                .contains("diagnostic-secret")
        );
        assert_eq!(
            error
                .source
                .unwrap()
                .downcast::<GuardrailTripwire>()
                .unwrap()
                .output,
            json!({"private": "diagnostic-secret"})
        );
    }
}

struct Check {
    name: &'static str,
    seen: Arc<Mutex<Vec<String>>>,
    output: Option<GuardrailResult>,
    panic: bool,
    fail: bool,
}
impl Guardrail for Check {
    fn name(&self) -> &str {
        self.name
    }
    fn check<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
        input: GuardrailInput<'a>,
    ) -> BoxFuture<'a, Result<Option<GuardrailResult>, Error>> {
        assert!(!self.panic, "synthetic callback construction panic");
        Box::pin(async move {
            let observed = match input {
                GuardrailInput::ToolOutput { output, .. } => {
                    format!("{}:{:?}", self.name, output.content)
                }
                _ => self.name.into(),
            };
            self.seen.lock().unwrap().push(observed);
            if self.fail {
                return Err(Error::new(ErrorCategory::Internal, "callback failed"));
            }
            Ok(self.output.clone())
        })
    }
}
fn context() -> Context {
    Context {
        run_id: "guards".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn guard(
    name: &'static str,
    seen: &Arc<Mutex<Vec<String>>>,
    output: Option<GuardrailResult>,
) -> Arc<dyn Guardrail> {
    Arc::new(Check {
        name,
        seen: seen.clone(),
        output,
        panic: false,
        fail: false,
    })
}

#[tokio::test]
async fn sequential_nil_and_tripwire_preserve_typed_results_and_stop_callbacks() {
    let seen = Arc::new(Mutex::new(vec![]));
    let guards = vec![
        guard("nil", &seen, None),
        guard(
            "block",
            &seen,
            Some(GuardrailResult {
                output: json!({"reason":"blocked"}),
                tripwire_triggered: true,
                replacement_content: None,
            }),
        ),
        guard("never", &seen, None),
    ];
    let result = run_guardrails(&guards, &context(), "agent", GuardrailInput::Input(&[])).await;
    assert!(result.tripped());
    assert_eq!(*seen.lock().unwrap(), vec!["nil", "block"]);
    assert_eq!(result.reports.len(), 2);
    assert!(result.reports[0].output.is_null());
    let cause = result
        .error
        .unwrap()
        .source
        .unwrap()
        .downcast::<GuardrailTripwire>()
        .unwrap();
    assert_eq!(cause.phase, GuardrailPhase::Input);
    assert_eq!(cause.output, json!({"reason":"blocked"}));
}

#[tokio::test]
async fn tool_output_replacement_precedes_next_guard_and_preserves_flags() {
    let seen = Arc::new(Mutex::new(vec![]));
    let guards = vec![
        guard(
            "sanitize",
            &seen,
            Some(GuardrailResult {
                output: json!("safe"),
                tripwire_triggered: false,
                replacement_content: Some("safe".into()),
            }),
        ),
        guard("inspect", &seen, None),
    ];
    let mut output = ToolOutput {
        content: vec![Content::Text {
            text: "original".into(),
        }],
        is_error: true,
        should_pause: true,
    };
    let call = ToolCall {
        raw_arguments: None,
        id: "1".into(),
        name: "tool".into(),
        arguments: json!({}),
    };
    let result = run_tool_output_guardrails(&guards, &context(), "agent", &call, &mut output).await;
    assert!(result.error.is_none());
    assert_eq!(
        output.content,
        vec![Content::Text {
            text: "safe".into()
        }]
    );
    assert!(output.is_error && output.should_pause);
    assert!(seen.lock().unwrap()[1].contains("safe"));
    assert!(!seen.lock().unwrap()[1].contains("original"));
    assert_eq!(result.reports[0].tool_name.as_deref(), Some("tool"));
}

#[tokio::test]
async fn callback_failure_and_panic_are_not_tripwires_or_partial_success() {
    let seen = Arc::new(Mutex::new(vec![]));
    for panic in [false, true] {
        let guards = vec![
            guard("first", &seen, None),
            Arc::new(Check {
                name: "failed",
                seen: seen.clone(),
                output: None,
                panic,
                fail: true,
            }) as Arc<dyn Guardrail>,
        ];
        let result = run_guardrails(
            &guards,
            &context(),
            "agent",
            GuardrailInput::Output(&json!("answer")),
        )
        .await;
        assert!(!result.tripped());
        assert!(result.reports.is_empty());
        assert_eq!(
            result.error.unwrap().info.category,
            ErrorCategory::Guardrail
        );
    }
}

#[tokio::test]
async fn cancellation_prevents_callback_invocation() {
    let token = CancellationToken::new();
    token.cancel();
    let context = Context {
        cancellation: Arc::new(token),
        ..context()
    };
    let seen = Arc::new(Mutex::new(vec![]));
    let result = run_guardrails(
        &[guard("never", &seen, None)],
        &context,
        "agent",
        GuardrailInput::Input(&[]),
    )
    .await;
    assert_eq!(
        result.error.unwrap().info.category,
        ErrorCategory::Cancelled
    );
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn diagnostic_strings_do_not_replace_content_and_tripwire_precedes_replacement() {
    let seen = Arc::new(Mutex::new(vec![]));
    let call = ToolCall {
        raw_arguments: None,
        id: "1".into(),
        name: "tool".into(),
        arguments: json!({}),
    };
    for trip in [false, true] {
        let guards = vec![guard(
            "inspect",
            &seen,
            Some(GuardrailResult {
                output: json!("diagnostic"),
                tripwire_triggered: trip,
                replacement_content: trip.then(|| "replacement".into()),
            }),
        )];
        let original = ToolOutput {
            content: vec![Content::Text {
                text: "original".into(),
            }],
            is_error: false,
            should_pause: false,
        };
        let mut output = original.clone();
        let result =
            run_tool_output_guardrails(&guards, &context(), "agent", &call, &mut output).await;
        assert_eq!(result.tripped(), trip);
        assert_eq!(output, original);
        assert_eq!(result.reports[0].output, json!("diagnostic"));
    }
    let guards = vec![guard(
        "clear",
        &seen,
        Some(GuardrailResult {
            replacement_content: Some(String::new()),
            ..Default::default()
        }),
    )];
    let mut output = ToolOutput {
        content: vec![Content::Text {
            text: "original".into(),
        }],
        is_error: false,
        should_pause: true,
    };
    assert!(
        run_tool_output_guardrails(&guards, &context(), "agent", &call, &mut output)
            .await
            .error
            .is_none()
    );
    assert_eq!(
        output.content,
        vec![Content::Text {
            text: String::new()
        }]
    );
    assert!(output.should_pause);
}
