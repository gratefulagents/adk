use adk_core::{BoxFuture, Content, Context, Error, ErrorCategory};
use adk_runtime::{Guardrail, GuardrailInput, GuardrailResult};
use regex::Regex;
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct GuardrailRule {
    pub name: String,
    pub regex: String,
    pub action: String,
    pub rule_type: String,
    pub tool_pattern: String,
    pub message: String,
}
#[derive(Default)]
pub struct CompiledGuardrails {
    pub input: Vec<Arc<dyn Guardrail>>,
    pub output: Vec<Arc<dyn Guardrail>>,
}
/// Native tool input is canonical JSON, not the source raw JSON byte spelling.
/// Compile before any model execution. This is the shared Go/Rust regex subset,
/// not a claim that Rust's regex parser implements RE2. Go-only escapes (e.g.
/// `\Q...\E`, `\C`) fail explicitly. Perl classes/boundaries inside character
/// classes are rejected; outside classes their ASCII Go semantics are preserved.
pub fn compile_guardrail_rules(rules: &[GuardrailRule]) -> Result<CompiledGuardrails, Error> {
    let mut compiled = CompiledGuardrails::default();
    let rust_only_flags = Regex::new(r"\(\?[a-zA-Z-]*[xuR]").expect("static regex");
    for rule in rules {
        if rust_only_flags.is_match(&rule.regex) {
            return Err(Error::new(
                ErrorCategory::Unsupported,
                format!(
                    "guardrail {:?}: Rust-only regex flags are not Go compatible",
                    rule.name
                ),
            ));
        }
        let mut pattern = String::new();
        let mut chars = rule.regex.chars().peekable();
        let mut in_class = false;
        let mut class_has_atom = false;
        let mut class_can_negate = false;
        while let Some(c) = chars.next() {
            if c == '\\' {
                if in_class {
                    class_has_atom = true;
                    class_can_negate = false;
                }
                let Some(escaped) = chars.next() else {
                    pattern.push(c);
                    break;
                };
                if "dDsSwWbB".contains(escaped) {
                    if in_class {
                        return Err(Error::new(
                            ErrorCategory::Unsupported,
                            format!(
                                "guardrail {:?}: Go/Rust regex syntax gap: Perl classes inside character classes",
                                rule.name
                            ),
                        ));
                    }
                    pattern.push_str(match escaped {
                        'd' => "[0-9]",
                        'D' => "[^0-9]",
                        's' => "[\\t\\n\\f\\r ]",
                        'S' => "[^\\t\\n\\f\\r ]",
                        'w' => "[A-Za-z0-9_]",
                        'W' => "[^A-Za-z0-9_]",
                        'b' => "(?-u:\\b)",
                        'B' => "(?-u:\\B)",
                        _ => unreachable!(),
                    });
                } else {
                    pattern.push(c);
                    pattern.push(escaped);
                }
            } else {
                if c == '[' {
                    if in_class {
                        return Err(Error::new(
                            ErrorCategory::Unsupported,
                            "Go/Rust nested character-class syntax differs (including POSIX classes)",
                        ));
                    }
                    in_class = true;
                    class_has_atom = false;
                    class_can_negate = true;
                } else if in_class {
                    if matches!(c, '&' | '~' | '-' | '|') && chars.peek() == Some(&c) {
                        return Err(Error::new(
                            ErrorCategory::Unsupported,
                            "Go/Rust regex character-class set syntax differs",
                        ));
                    }
                    if c == ']' && class_has_atom {
                        in_class = false;
                    } else if c == '^' && class_can_negate {
                        class_can_negate = false;
                    } else {
                        class_has_atom = true;
                        class_can_negate = false;
                    }
                }
                pattern.push(c);
            }
        }
        let regex = Regex::new(&pattern).map_err(|error| {
            Error::new(
                ErrorCategory::InvalidInput,
                format!(
                    "invalid or unsupported Go/Rust regex in guardrail {:?}: {error}",
                    rule.name
                ),
            )
        })?;
        let action = rule.action.trim().to_ascii_lowercase();
        let action = if action.is_empty() {
            "block".into()
        } else {
            action
        };
        if !matches!(action.as_str(), "block" | "warn" | "log") {
            return Err(Error::new(
                ErrorCategory::InvalidInput,
                format!(
                    "unknown action {:?} in guardrail {:?}",
                    rule.action, rule.name
                ),
            ));
        }
        let output = match rule.rule_type.as_str() {
            "tool-input" => false,
            "tool-output" => true,
            _ => {
                return Err(Error::new(
                    ErrorCategory::InvalidInput,
                    format!(
                        "unknown guardrail rule type {:?} for {:?}",
                        rule.rule_type, rule.name
                    ),
                ));
            }
        };
        let guard = Arc::new(Rule {
            rule: rule.clone(),
            name: format!("config:{}", rule.name),
            regex,
            action,
            output,
        });
        if output {
            compiled.output.push(guard);
        } else {
            compiled.input.push(guard);
        }
    }
    Ok(compiled)
}
struct Rule {
    rule: GuardrailRule,
    name: String,
    regex: Regex,
    action: String,
    output: bool,
}
impl Guardrail for Rule {
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
                GuardrailInput::ToolInput(call) => (&call.name, call.arguments.to_string()),
                GuardrailInput::ToolOutput { call, output } => (
                    &call.name,
                    output
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            Content::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                _ => return Ok(None),
            };
            if (!self.rule.tool_pattern.is_empty() && !matches_tool(name, &self.rule.tool_pattern))
                || !self.regex.is_match(&text)
            {
                return Ok(Some(GuardrailResult::default()));
            }
            let message = if self.rule.message.trim().is_empty() {
                format!(
                    "Guardrail {:?} triggered on tool {:?}{}",
                    self.rule.name,
                    name,
                    if self.output { " output" } else { "" }
                )
            } else {
                self.rule.message.trim().to_owned()
            };
            if self.action != "block" {
                eprintln!(
                    "{}: guardrail {:?} triggered: {}",
                    if self.action == "warn" {
                        "WARN"
                    } else {
                        "INFO"
                    },
                    self.rule.name,
                    message
                );
            }
            Ok(Some(GuardrailResult {
                output: if self.action == "log" {
                    serde_json::Value::Null
                } else {
                    message.into()
                },
                tripwire_triggered: self.action == "block",
                replacement_content: None,
            }))
        })
    }
}
fn matches_tool(name: &str, pattern: &str) -> bool {
    let segments: Vec<_> = pattern.split('*').collect();
    if segments.len() == 1 {
        return name == pattern;
    }
    let Some(mut rest) = name.strip_prefix(segments[0]) else {
        return false;
    };
    for segment in &segments[1..segments.len() - 1] {
        let Some(index) = rest.find(segment) else {
            return false;
        };
        rest = &rest[index + segment.len()..];
    }
    rest.ends_with(segments[segments.len() - 1])
}
