use super::match_tool_pattern;
use crate::{content_logger::quote, host::GuardrailRule};
use adk_core::{BoxFuture, Content, Context, Error};
use adk_runtime::{Guardrail, GuardrailInput, GuardrailResult};
use regex::Regex;
use std::sync::Arc;

/// Construct a public SDK-style input rule with a caller-compiled Rust regex.
/// The supplied regex, not `rule.regex`, is used. Actions are not normalized:
/// only exact `warn` and `log` avoid blocking. This is not a Go regex compiler.
pub fn make_tool_input_guardrail(rule: GuardrailRule, regex: Regex) -> Arc<dyn Guardrail> {
    Arc::new(PublicRule {
        name: format!("crd:{}", rule.name),
        rule,
        regex,
        output: false,
    })
}

/// Construct a public SDK-style output rule over newline-joined text parts.
/// Uses the supplied Rust regex and the same unnormalized action semantics as
/// [`make_tool_input_guardrail`]. Non-text output parts are not matched.
pub fn make_tool_output_guardrail(rule: GuardrailRule, regex: Regex) -> Arc<dyn Guardrail> {
    Arc::new(PublicRule {
        name: format!("crd:{}", rule.name),
        rule,
        regex,
        output: true,
    })
}

struct PublicRule {
    name: String,
    rule: GuardrailRule,
    regex: Regex,
    output: bool,
}

impl Guardrail for PublicRule {
    fn name(&self) -> &str {
        &self.name
    }

    fn check<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
        input: GuardrailInput<'a>,
    ) -> BoxFuture<'a, Result<Option<GuardrailResult>, Error>> {
        Box::pin(async move {
            let (name, text) = match input {
                GuardrailInput::ToolInput(call) if !self.output => {
                    (&call.name, call.argument_text())
                }
                GuardrailInput::ToolOutput { call, output } if self.output => (
                    &call.name,
                    output
                        .content
                        .iter()
                        .filter_map(|part| match part {
                            Content::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                        .into(),
                ),
                _ => return Ok(None),
            };
            if (!self.rule.tool_pattern.is_empty()
                && !match_tool_pattern(name, &self.rule.tool_pattern))
                || !self.regex.is_match(&text)
            {
                return Ok(Some(GuardrailResult::default()));
            }
            let message = if self.rule.message.is_empty() {
                format!(
                    "Guardrail {} triggered on tool {}{}",
                    quote(self.rule.name.as_bytes()),
                    quote(name.as_bytes()),
                    if self.output { " output" } else { "" }
                )
            } else {
                self.rule.message.clone()
            };
            if matches!(self.rule.action.as_str(), "warn" | "log") {
                eprintln!(
                    "{}: guardrail {} triggered: {}",
                    if self.rule.action == "warn" {
                        "WARN"
                    } else {
                        "INFO"
                    },
                    quote(self.rule.name.as_bytes()),
                    message
                );
            }
            Ok(Some(GuardrailResult {
                output: if self.rule.action == "log" {
                    serde_json::Value::Null
                } else {
                    message.into()
                },
                tripwire_triggered: !matches!(self.rule.action.as_str(), "warn" | "log"),
                replacement_content: None,
            }))
        })
    }
}
