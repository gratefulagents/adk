use super::match_tool_pattern;
use crate::{
    content_logger::quote,
    host::{CompiledGuardrails, GuardrailRule, compile_rule_regex},
};
use adk_core::{BoxFuture, Content, Context, Error, ErrorCategory};
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

/// Compile public SDK-style rules, retaining valid rules and collecting errors in
/// source order. Callers must handle the errors before using a partial rule set.
/// Uses the host compiler's explicit shared Go/Rust regex subset, not full RE2.
/// Unlike direct constructors, this normalizes action case and whitespace.
#[must_use]
pub fn tool_guardrails_from_rules(rules: &[GuardrailRule]) -> (CompiledGuardrails, Vec<Error>) {
    let mut compiled = CompiledGuardrails::default();
    let mut errors = Vec::new();
    for rule in rules {
        let regex = match compile_rule_regex(rule) {
            Ok(regex) => regex,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        let mut rule = rule.clone();
        let action = rule.action.trim().to_lowercase();
        rule.action = match action.as_str() {
            "" => "block".into(),
            "block" | "warn" | "log" => action,
            _ => {
                errors.push(Error::new(
                    ErrorCategory::InvalidInput,
                    format!(
                        "unknown action {} in guardrail rule {} (valid: block, warn, log)",
                        quote(rule.action.as_bytes()),
                        quote(rule.name.as_bytes())
                    ),
                ));
                continue;
            }
        };
        match rule.rule_type.as_str() {
            "tool-input" => compiled.input.push(make_tool_input_guardrail(rule, regex)),
            "tool-output" => compiled
                .output
                .push(make_tool_output_guardrail(rule, regex)),
            _ => errors.push(Error::new(
                ErrorCategory::InvalidInput,
                format!(
                    "unknown guardrail rule type {} for rule {} (valid: tool-input, tool-output)",
                    quote(rule.rule_type.as_bytes()),
                    quote(rule.name.as_bytes())
                ),
            )),
        }
    }
    (compiled, errors)
}
