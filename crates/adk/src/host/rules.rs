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
/// their negations are accepted. Perl class atoms retain ASCII Go semantics
/// inside and outside character classes; in-class word boundaries are rejected.
/// Hyphen runs follow Go character ranges, not Rust class-set subtraction.
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
    static GO_REPEAT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:0|[1-9][0-9]*)(?:,(?:0|[1-9][0-9]*)?)?\}").expect("static regex")
    });
    let mut pattern = String::new();
    let mut chars = rule.regex.chars().peekable();
    let mut in_class = false;
    let mut class_has_atom = false;
    let mut class_can_negate = false;
    let mut class_has_char = false;
    let mut class_range_end = false;
    while let Some(c) = chars.next() {
        if c == '\\' {
            let range_end = in_class && class_range_end;
            if in_class {
                class_has_char = !class_range_end;
                class_range_end = false;
                class_has_atom = true;
                class_can_negate = false;
            }
            let Some(escaped) = chars.next() else {
                pattern.push(c);
                break;
            };
            if matches!(escaped, '0'..='7')
                && (escaped == '0' || matches!(chars.peek(), Some('0'..='7')))
            {
                let mut value = escaped as u32 - '0' as u32;
                for _ in 0..2 {
                    if let Some(digit @ '0'..='7') = chars.peek().copied() {
                        chars.next();
                        value = value * 8 + digit as u32 - '0' as u32;
                    } else {
                        break;
                    }
                }
                pattern.push_str(&format!(r"\x{{{value:x}}}"));
                continue;
            }
            if in_class && matches!(escaped, 'p' | 'P' | 'd' | 'D' | 's' | 'S' | 'w' | 'W') {
                if range_end {
                    return Err(Error::new(
                        ErrorCategory::InvalidInput,
                        "Go character-class range endpoints must be single characters",
                    ));
                }
                class_has_char = false;
            }
            if matches!(escaped, 'p' | 'P') {
                let mut name = String::new();
                if chars.peek() == Some(&'{') {
                    chars.next();
                    let mut closed = false;
                    for operand in chars.by_ref() {
                        if operand == '}' {
                            closed = true;
                            break;
                        }
                        name.push(operand);
                    }
                    if !closed {
                        return Err(Error::new(
                            ErrorCategory::InvalidInput,
                            "unclosed Unicode class",
                        ));
                    }
                } else if let Some(operand) = chars.next() {
                    name.push(operand);
                }
                let negated = (escaped == 'P') ^ name.starts_with('^');
                let name: String = name
                    .strip_prefix('^')
                    .unwrap_or(&name)
                    .chars()
                    .filter(|c| !matches!(c, '_' | '-' | ' '))
                    .map(|c| c.to_ascii_lowercase())
                    .collect();
                // Rust also accepts binary properties and property=value syntax
                // that the pinned Go compiler rejects.
                if !include_str!("go_unicode_names.txt")
                    .lines()
                    .any(|n| n == name)
                {
                    return Err(Error::new(
                        ErrorCategory::InvalidInput,
                        "invalid Go Unicode class",
                    ));
                }
                // Surrogates cannot occur in UTF-8 input; Rust has no Cs table.
                if matches!(name.as_str(), "cs" | "surrogate") {
                    pattern.push_str(if negated { r"[\s\S]" } else { r"[^\s\S]" });
                    continue;
                }
                pattern.push_str(if negated { r"\P{" } else { r"\p{" });
                pattern.push_str(&name);
                pattern.push('}');
                continue;
            }
            if escaped == 'x' && chars.peek() != Some(&'{') {
                pattern.push(c);
                pattern.push(escaped);
                for operand in chars.by_ref().take(2) {
                    pattern.push(operand);
                }
                continue;
            }
            if escaped == 'x' && chars.peek() == Some(&'{') {
                pattern.push(c);
                pattern.push(escaped);
                for argument in chars.by_ref() {
                    pattern.push(argument);
                    if argument == '}' {
                        break;
                    }
                }
                continue;
            }
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
                if in_class && matches!(escaped, 'b' | 'B') {
                    return Err(Error::new(
                        ErrorCategory::InvalidInput,
                        format!(
                            "guardrail {:?}: Go does not permit word boundaries inside character classes",
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
            if c == '(' && !in_class {
                let prefix: String = chars.clone().take(3).collect();
                let prefix_len = if prefix.starts_with("?P<") {
                    3
                } else if prefix.starts_with("?<") {
                    2
                } else {
                    0
                };
                if prefix_len > 0 {
                    for _ in 0..prefix_len {
                        chars.next();
                    }
                    let mut name = String::new();
                    let mut closed = false;
                    for character in chars.by_ref() {
                        if character == '>' {
                            closed = true;
                            break;
                        }
                        name.push(character);
                    }
                    if !closed
                        || name.is_empty()
                        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        return Err(Error::new(
                            ErrorCategory::InvalidInput,
                            format!("invalid Go capture name in guardrail {:?}", rule.name),
                        ));
                    }
                    // Guardrails expose only membership, not captures. Erasing the
                    // validated name permits Go's digit-leading and duplicate names.
                    pattern.push_str("(?:");
                    continue;
                }
            }
            if c == '{'
                && !in_class
                && !GO_REPEAT.is_match(
                    &chars
                        .clone()
                        .take_while(|c| c.is_ascii_digit() || matches!(c, ',' | '}'))
                        .collect::<String>(),
                )
            {
                // Go parses malformed counts (including leading zeros) as literals.
                pattern.push('\\');
            }
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
                    if !class_range_end {
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
                            class_has_char = false;
                            class_has_atom = true;
                            class_can_negate = false;
                            continue;
                        }
                        if chars.peek() == Some(&':')
                            && chars.clone().skip(1).collect::<String>().contains(":]")
                        {
                            return Err(Error::new(
                                ErrorCategory::InvalidInput,
                                "invalid Go POSIX character-class name",
                            ));
                        }
                    }
                    pattern.push_str(r"\[");
                    class_has_char = !class_range_end;
                    class_range_end = false;
                    class_has_atom = true;
                    class_can_negate = false;
                    continue;
                }
                in_class = true;
                class_has_atom = false;
                class_can_negate = true;
                class_has_char = false;
                class_range_end = false;
            } else if in_class {
                if c == ']' && !class_has_atom {
                    pattern.push_str(r"\x5d");
                    class_has_char = true;
                    class_has_atom = true;
                    class_can_negate = false;
                    continue;
                }
                if matches!(c, '&' | '~' | '|') {
                    pattern.push('\\');
                }
                if c == '-' {
                    if class_has_char && chars.peek() != Some(&']') {
                        pattern.push('-');
                        class_has_char = false;
                        class_range_end = true;
                    } else {
                        // Go treats this as a character, not Rust set subtraction.
                        pattern.push_str(r"\x2d");
                        class_has_char = !class_range_end;
                        class_range_end = false;
                    }
                    class_has_atom = true;
                    class_can_negate = false;
                    continue;
                }
                if c == ']' && class_has_atom {
                    in_class = false;
                } else if c == '^' && class_can_negate {
                    class_can_negate = false;
                } else {
                    class_has_char = !class_range_end;
                    class_range_end = false;
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
