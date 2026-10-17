//! Opt-in tool guards. Native shell and secret normalization are stricter than
//! the SDK scalar-text guards; these checks are not an execution sandbox.
use adk_core::{BoxFuture, Content, Context, Error};
use adk_runtime::{Guardrail, GuardrailInput, GuardrailResult};
use adk_security::{
    ToolOutputDisposition, check_destructive_command, check_secrets, sanitize_tool_output,
};
use serde::Deserialize;
use std::sync::Arc;

/// Match the SDK public helper's edge-star syntax, not ChatLoop's multi-star glob.
/// A trailing star takes precedence over a leading star; internal stars are literal.
pub fn match_tool_pattern(name: &str, pattern: &str) -> bool {
    if pattern == "*" {
        true
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else if let Some(suffix) = pattern.strip_prefix('*') {
        name.ends_with(suffix)
    } else {
        name == pattern
    }
}

pub fn builtin_tool_input_guardrails() -> Vec<Arc<dyn Guardrail>> {
    vec![
        Arc::new(Builtin::Destructive),
        Arc::new(Builtin::SecretInput),
    ]
}

pub fn builtin_tool_output_guardrails() -> Vec<Arc<dyn Guardrail>> {
    vec![Arc::new(Builtin::SecretOutput)]
}

enum Builtin {
    Destructive,
    SecretInput,
    SecretOutput,
}

#[derive(Default)]
struct ShellArguments {
    command: String,
    cmd: String,
}

impl<'de> Deserialize<'de> for ShellArguments {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = ShellArguments;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("shell arguments object")
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                let mut arguments = ShellArguments::default();
                while let Some(key) = map.next_key::<String>()? {
                    let field = if key.eq_ignore_ascii_case("command") {
                        &mut arguments.command
                    } else if key.eq_ignore_ascii_case("cmd") {
                        &mut arguments.cmd
                    } else {
                        map.next_value::<serde::de::IgnoredAny>()?;
                        continue;
                    };
                    if let Some(value) = map.next_value::<Option<String>>()? {
                        *field = value;
                    }
                }
                Ok(arguments)
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

fn blocked(message: impl Into<String>) -> GuardrailResult {
    GuardrailResult {
        output: message.into().into(),
        tripwire_triggered: true,
        ..Default::default()
    }
}

impl Guardrail for Builtin {
    fn name(&self) -> &str {
        match self {
            Self::Destructive => "block-destructive-commands",
            Self::SecretInput => "detect-secret-leak",
            Self::SecretOutput => "detect-secret-in-output",
        }
    }

    fn durable_key(&self) -> Option<&str> {
        Some(match self {
            Self::Destructive => "adk.builtin.block-destructive-commands.v2",
            Self::SecretInput => "adk.builtin.detect-secret-leak.v2",
            Self::SecretOutput => "adk.builtin.detect-secret-in-output.v1",
        })
    }

    fn check<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
        input: GuardrailInput<'a>,
    ) -> BoxFuture<'a, Result<Option<GuardrailResult>, Error>> {
        Box::pin(async move {
            let result = match (self, input) {
                (Self::Destructive, GuardrailInput::ToolInput(call)) => {
                    let name = call.name.to_ascii_lowercase();
                    if !["bash", "shell", "exec"]
                        .iter()
                        .any(|part| name.contains(part))
                    {
                        return Ok(None);
                    }
                    let Ok(params) = adk_codec::userinput::decode::<Option<ShellArguments>>(
                        call.argument_text().as_bytes(),
                    ) else {
                        return Ok(Some(blocked(
                            "Cannot parse shell tool input for destructive-command check",
                        )));
                    };
                    let params = params.unwrap_or_default();
                    let command = if params.command.is_empty() {
                        params.cmd
                    } else {
                        params.command
                    };
                    if command.is_empty() {
                        return Ok(None);
                    }
                    check_destructive_command(&command).err().map(|_| {
                        blocked("Shell command is destructive or cannot be classified safely")
                    })
                }
                (Self::SecretInput, GuardrailInput::ToolInput(call)) => {
                    // JSON escaping must not hide credentials in decoded argument strings.
                    fn scan(value: &serde_json::Value) -> Result<(), Error> {
                        match value {
                            serde_json::Value::String(s) => check_secrets(s),
                            serde_json::Value::Array(values) => values.iter().try_for_each(scan),
                            serde_json::Value::Object(values) => {
                                values.iter().try_for_each(|(key, value)| {
                                    check_secrets(key)?;
                                    scan(value)
                                })
                            }
                            _ => Ok(()),
                        }
                    }
                    check_secrets(&call.argument_text())
                        .and_then(|_| scan(&call.arguments))
                        .err()
                        .map(|error| blocked(error.to_string()))
                }
                (Self::SecretOutput, GuardrailInput::ToolOutput { output, .. }) => {
                    let parts = output
                        .content
                        .iter()
                        .filter_map(|part| match part {
                            Content::Text { text } | Content::Reasoning { text, .. } => {
                                Some(text.as_str())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    let text = parts.join("");
                    // Rendered boundaries and split credentials require different scan views.
                    if parts.len() > 1
                        && parts
                            .iter()
                            .copied()
                            .chain([parts.join("\n").as_str(), text.as_str()])
                            .any(|part| check_secrets(part).is_err())
                    {
                        return Ok(Some(blocked(
                            "Secret detected in multipart tool output; output blocked",
                        )));
                    }
                    match sanitize_tool_output(&text) {
                        ToolOutputDisposition::Unchanged => None,
                        ToolOutputDisposition::Blocked { kind } => Some(blocked(format!(
                            "Potential {} detected in tool output; output blocked because companion credentials may remain",
                            kind.0
                        ))),
                        ToolOutputDisposition::Redacted { content, notice } => {
                            if output
                                .content
                                .iter()
                                .any(|part| !matches!(part, Content::Text { .. }))
                            {
                                Some(blocked(
                                    "Secret detected in non-text or mixed-content tool output; output blocked",
                                ))
                            } else {
                                Some(GuardrailResult {
                                    output: notice.into(),
                                    replacement_content: Some(content),
                                    ..Default::default()
                                })
                            }
                        }
                    }
                }
                _ => None,
            };
            Ok(result)
        })
    }
}
