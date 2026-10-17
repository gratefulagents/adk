use adk_core::{BoxFuture, Content, Context, Error, ErrorCategory};
use adk_runtime::{Guardrail, GuardrailInput, GuardrailResult};
use regex::Regex;
use std::sync::{Arc, LazyLock};

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
/// Input rules match original UTF-8 argument text when available.
/// Compile before any model execution. This is the shared Go/Rust regex subset,
/// not a claim that Rust's regex parser implements RE2. Quoted `\Q...\E`
/// literals are translated outside classes. Named POSIX ASCII class atoms and
/// their negations are accepted. Perl classes/boundaries inside character
/// classes are rejected; outside classes their ASCII Go semantics are preserved.
pub fn compile_guardrail_rules(rules: &[GuardrailRule]) -> Result<CompiledGuardrails, Error> {
    let mut compiled = CompiledGuardrails::default();
    for rule in rules {
        let regex = compile_rule_regex(rule)?;
        let action = rule.action.trim().to_lowercase();
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
                GuardrailInput::ToolInput(call) => (&call.name, call.argument_text()),
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
                        .join("\n")
                        .into(),
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

pub(crate) fn compile_rule_regex(rule: &GuardrailRule) -> Result<Regex, Error> {
    static RUST_ONLY_FLAGS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\(\?[a-zA-Z-]*[xuR]").expect("static regex"));
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
            if escaped == 'Q' && !in_class {
                let mut literal = String::new();
                while let Some(quoted) = chars.next() {
                    if quoted == '\\' && chars.peek() == Some(&'E') {
                        chars.next();
                        break;
                    }
                    literal.push(quoted);
                }
                pattern.push_str(&regex::escape(&literal));
                continue;
            }
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
            if c == '('
                && !in_class
                && RUST_ONLY_FLAGS.is_match(&format!(
                    "({}",
                    chars
                        .clone()
                        .take_while(|c| c.is_ascii_alphabetic() || matches!(c, '?' | '-'))
                        .collect::<String>()
                ))
            {
                return Err(Error::new(
                    ErrorCategory::Unsupported,
                    format!(
                        "guardrail {:?}: Rust-only regex flags are not Go compatible",
                        rule.name
                    ),
                ));
            }
            if c == '[' {
                if in_class {
                    // POSIX ASCII atoms are shared syntax, unlike arbitrary nested
                    // classes or Rust set operations. Consume only an exact Go name.
                    let candidate: String = chars.clone().take(10).collect();
                    let class = candidate
                        .strip_prefix(':')
                        .and_then(|suffix| suffix.split_once(":]").map(|(name, _)| name));
                    if let Some(class) = class.filter(|class| {
                        matches!(
                            class.strip_prefix('^').unwrap_or(class),
                            "alnum"
                                | "alpha"
                                | "ascii"
                                | "blank"
                                | "cntrl"
                                | "digit"
                                | "graph"
                                | "lower"
                                | "print"
                                | "punct"
                                | "space"
                                | "upper"
                                | "word"
                                | "xdigit"
                        )
                    }) {
                        pattern.push(c);
                        pattern.extend(chars.by_ref().take(class.len() + 3));
                        class_has_atom = true;
                        class_can_negate = false;
                        continue;
                    }
                    return Err(Error::new(
                        ErrorCategory::Unsupported,
                        "Go/Rust nested character-class syntax differs outside recognized POSIX atoms",
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
    use regex_syntax::ast::{Ast, RepetitionKind, RepetitionRange};
    let ast = regex_syntax::ast::parse::Parser::new()
        .parse(&pattern)
        .map_err(|error| {
            Error::new(
                ErrorCategory::InvalidInput,
                format!(
                    "invalid or unsupported Go/Rust regex in guardrail {:?}: {error}",
                    rule.name
                ),
            )
        })?;
    let mut pending = vec![(&ast, 1000)];
    while let Some((node, mut limit)) = pending.pop() {
        match node {
            Ast::Repetition(repetition) => {
                if let RepetitionKind::Range(range) = &repetition.op.kind {
                    let count = match *range {
                        RepetitionRange::Exactly(count) | RepetitionRange::AtLeast(count) => count,
                        RepetitionRange::Bounded(_, max) => max,
                    };
                    if count > limit {
                        return Err(Error::new(
                            ErrorCategory::InvalidInput,
                            format!("invalid Go repeat count in guardrail {:?}", rule.name),
                        ));
                    }
                    if matches!(
                        range,
                        RepetitionRange::Exactly(0) | RepetitionRange::Bounded(_, 0)
                    ) {
                        // A zero maximum shields its subtree from outer products,
                        // but inner repetitions must still be independently valid.
                        limit = 1000;
                    } else if count > 0 {
                        limit /= count;
                    }
                }
                pending.push((&repetition.ast, limit));
            }
            Ast::Group(group) => pending.push((&group.ast, limit)),
            Ast::Alternation(alternation) => {
                pending.extend(alternation.asts.iter().map(|node| (node, limit)));
            }
            Ast::Concat(concat) => pending.extend(concat.asts.iter().map(|node| (node, limit))),
            _ => {}
        }
    }
    Regex::new(&pattern).map_err(|error| {
        Error::new(
            ErrorCategory::InvalidInput,
            format!(
                "invalid or unsupported Go/Rust regex in guardrail {:?}: {error}",
                rule.name
            ),
        )
    })
}
