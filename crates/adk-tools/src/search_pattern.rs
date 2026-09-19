#[derive(Clone)]
enum Token {
    Literal(char),
    Any,
    Star,
    Class(bool, Vec<(char, char)>),
}
#[derive(Clone)]
pub(crate) struct Pattern {
    parts: Vec<Option<Vec<Token>>>,
}

impl Pattern {
    pub fn new(pattern: &str) -> Result<Self, String> {
        if pattern.len() > 4096 {
            return Err("pattern exceeds 4096 bytes".into());
        }
        let mut parts = Vec::new();
        for part in pattern.split('/') {
            if part == "**" {
                parts.push(None);
                continue;
            }
            let mut chars = part.chars().peekable();
            let mut tokens = Vec::new();
            while let Some(c) = chars.next() {
                tokens.push(match c {
                    '*' => Token::Star,
                    '?' => Token::Any,
                    '\\' => Token::Literal(chars.next().ok_or("syntax error in pattern")?),
                    '[' => {
                        let negate = chars.peek() == Some(&'^');
                        if negate {
                            chars.next();
                        }
                        let mut ranges = Vec::new();
                        loop {
                            if !ranges.is_empty() && chars.peek() == Some(&']') {
                                chars.next();
                                break;
                            }
                            let mut escaped = || -> Result<char, String> {
                                match chars.next() {
                                    Some('\\') => {
                                        chars.next().ok_or("syntax error in pattern".into())
                                    }
                                    Some(']' | '-') | None => Err("syntax error in pattern".into()),
                                    Some(c) => Ok(c),
                                }
                            };
                            let lo = escaped()?;
                            let hi = if chars.peek() == Some(&'-') {
                                chars.next();
                                match chars.next() {
                                    Some('\\') => chars.next().ok_or("syntax error in pattern")?,
                                    Some(']' | '-') | None => {
                                        return Err("syntax error in pattern".into());
                                    }
                                    Some(c) => c,
                                }
                            } else {
                                lo
                            };
                            ranges.push((lo, hi));
                        }
                        Token::Class(negate, ranges)
                    }
                    c => Token::Literal(c),
                });
            }
            parts.push(Some(tokens));
        }
        Ok(Self { parts })
    }

    pub fn matches(&self, path: &str) -> bool {
        let names: Vec<_> = path.split('/').collect();
        let mut state = vec![false; names.len() + 1];
        state[0] = true;
        for part in &self.parts {
            if let Some(tokens) = part {
                let mut next = vec![false; state.len()];
                for (i, name) in names.iter().enumerate() {
                    next[i + 1] = state[i] && component_matches(tokens, name);
                }
                state = next;
            } else {
                for i in 1..state.len() {
                    state[i] |= state[i - 1];
                }
            }
        }
        state[names.len()]
    }

    pub fn matches_path_or_basename(&self, path: &str) -> bool {
        self.matches(path) || self.matches(path.rsplit('/').next().unwrap_or(path))
    }
}

fn component_matches(tokens: &[Token], value: &str) -> bool {
    let chars: Vec<_> = value.chars().collect();
    let mut state = vec![false; chars.len() + 1];
    state[0] = true;
    for token in tokens {
        if matches!(token, Token::Star) {
            for i in 1..state.len() {
                state[i] |= state[i - 1];
            }
        } else {
            let mut next = vec![false; state.len()];
            for (i, c) in chars.iter().enumerate() {
                next[i + 1] = state[i]
                    && match token {
                        Token::Literal(want) => want == c,
                        Token::Any => true,
                        Token::Class(negate, ranges) => {
                            ranges.iter().any(|(lo, hi)| lo <= c && c <= hi) != *negate
                        }
                        Token::Star => unreachable!(),
                    };
            }
            state = next;
        }
    }
    state[chars.len()]
}

pub(crate) fn go_regex(pattern: &str, ignore_case: bool) -> Result<regex::Regex, String> {
    let mut translated = if ignore_case {
        "(?i)".to_owned()
    } else {
        String::new()
    };
    let mut chars = pattern.chars().peekable();
    let mut in_class = false;
    let mut class_first = false;
    while let Some(c) = chars.next() {
        if c == '[' {
            if in_class {
                if chars.peek() == Some(&':') {
                    translated.push('[');
                    for inner in chars.by_ref() {
                        translated.push(inner);
                        if inner == ']' {
                            break;
                        }
                    }
                } else {
                    translated.push_str("\\[");
                }
                class_first = false;
                continue;
            }
            in_class = true;
            class_first = true;
            translated.push('[');
            continue;
        }
        if c == ']' && in_class {
            if class_first {
                translated.push_str("\\]");
                class_first = false;
            } else {
                translated.push(']');
                in_class = false;
            }
            continue;
        }
        if in_class && matches!(c, '&' | '~') {
            translated.push_str(if c == '&' { "\\x26" } else { "\\x7e" });
            class_first = false;
            continue;
        }
        if !in_class && c == '(' && chars.peek() == Some(&'?') {
            let mut flags = chars.clone();
            flags.next();
            if !matches!(flags.peek(), Some(':' | '<' | 'P')) {
                for flag in flags {
                    if matches!(flag, ':' | ')') {
                        break;
                    }
                    if !matches!(flag, 'i' | 'm' | 's' | 'U' | '-') {
                        return Err("invalid or unsupported Perl syntax".into());
                    }
                }
            }
        }
        if !in_class && c == '{' {
            let content: String = chars.clone().take_while(|c| *c != '}').collect();
            let parts: Vec<_> = content.split(',').collect();
            let decimal = |part: &str| {
                !part.is_empty()
                    && part.bytes().all(|b| b.is_ascii_digit())
                    && (part.len() == 1 || !part.starts_with('0'))
            };
            if chars.clone().any(|c| c == '}')
                && parts.len() <= 2
                && decimal(parts[0])
                && (parts.len() == 1 || parts[1].is_empty() || decimal(parts[1]))
            {
                for part in &parts {
                    if !part.is_empty() && part.parse::<usize>().map_or(true, |n| n > 1000) {
                        return Err("invalid repeat count".into());
                    }
                }
                translated.push('{');
                for inner in chars.by_ref() {
                    translated.push(inner);
                    if inner == '}' {
                        break;
                    }
                }
            } else {
                translated.push_str("\\{");
            }
            continue;
        }
        if !in_class && c == '}' {
            translated.push_str("\\}");
            continue;
        }
        if in_class && !(class_first && c == '^') {
            class_first = false;
        }
        if c != '\\' {
            translated.push(c);
            continue;
        }
        let escaped = chars
            .next()
            .ok_or("trailing backslash at end of expression")?;
        match escaped {
            'Q' if !in_class => {
                let mut quoted = String::new();
                while let Some(c) = chars.next() {
                    if c == '\\' && chars.peek() == Some(&'E') {
                        chars.next();
                        break;
                    }
                    quoted.push(c);
                }
                translated.push_str(&regex::escape(&quoted));
            }
            'd' => translated.push_str("[0-9]"),
            'D' => translated.push_str("[^0-9]"),
            'w' => translated.push_str("[A-Za-z0-9_]"),
            'W' => translated.push_str("[^A-Za-z0-9_]"),
            's' => translated.push_str("[\\t\\n\\x0c\\r ]"),
            'S' => translated.push_str("[^\\t\\n\\x0c\\r ]"),
            'b' | 'B' if !in_class => {
                translated.push_str("(?-u:\\");
                translated.push(escaped);
                translated.push(')');
            }
            'p' | 'P' | 'x' => {
                translated.push('\\');
                translated.push(escaped);
                if chars.peek() == Some(&'{') {
                    translated.push(chars.next().unwrap());
                    for c in chars.by_ref() {
                        translated.push(c);
                        if c == '}' {
                            break;
                        }
                    }
                }
            }
            '0'..='7' => {
                if escaped != '0' && !chars.peek().is_some_and(|c| matches!(c, '0'..='7')) {
                    return Err("invalid escape sequence".into());
                }
                translated.push('\\');
                translated.push(escaped);
                for _ in 0..2 {
                    if chars.peek().is_some_and(|c| matches!(c, '0'..='7')) {
                        translated.push(chars.next().unwrap());
                    }
                }
            }
            'u' | 'U' => return Err("invalid escape sequence".into()),
            _ => {
                translated.push('\\');
                translated.push(escaped);
            }
        }
    }
    let hir = regex_syntax::ParserBuilder::new()
        .octal(true)
        .nest_limit(1000)
        .build()
        .parse(&translated)
        .map_err(|error| error.to_string())?;
    let mut stack = vec![(&hir, 1000u32)];
    while let Some((node, remaining)) = stack.pop() {
        use regex_syntax::hir::HirKind;
        match node.kind() {
            HirKind::Repetition(repeat) => {
                let count = repeat.max.unwrap_or(repeat.min);
                if repeat.max == Some(0) {
                    continue;
                }
                if count > remaining {
                    return Err("invalid repeat count".into());
                }
                stack.push((
                    &repeat.sub,
                    remaining.checked_div(count).unwrap_or(remaining),
                ));
            }
            HirKind::Capture(capture) => stack.push((&capture.sub, remaining)),
            HirKind::Concat(children) | HirKind::Alternation(children) => {
                stack.extend(children.iter().map(|child| (child, remaining)))
            }
            _ => {}
        }
    }
    regex::RegexBuilder::new(&translated)
        .octal(true)
        .nest_limit(1000)
        .size_limit(10 << 20)
        .build()
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn go_globs_and_double_stars() {
        for (pattern, value, want) in [
            ("**/*.rs", "main.rs", true),
            ("**/a/**/b", "a/b", true),
            ("[!a]", "!", true),
            ("[^a]", "b", true),
            ("[z-a]", "z", false),
            ("a?", "aé", true),
            ("*.rs", "src/a.rs", false),
            ("a\\*", "a*", true),
        ] {
            assert_eq!(
                Pattern::new(pattern).unwrap().matches(value),
                want,
                "{pattern} {value}"
            );
        }
        for pattern in ["[", "[]", "[a-]", "[-a]", "\\"] {
            assert!(Pattern::new(pattern).is_err(), "{pattern}");
        }
    }
    #[test]
    fn go_regex_ascii_classes_unicode_literals_and_quotes() {
        assert!(!go_regex(r"^\w+$", false).unwrap().is_match("é"));
        assert!(!go_regex(r"^\d+$", false).unwrap().is_match("١"));
        assert!(go_regex("^.$", false).unwrap().is_match("é"));
        assert!(go_regex(r"\Q[a].*\E", false).unwrap().is_match("[a].*"));
        assert!(go_regex(r"\bword\b", false).unwrap().is_match("éwordé"));
    }
}
