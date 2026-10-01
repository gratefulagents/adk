use adk_core::{Error, ErrorCategory};
use regex::Regex;
use std::sync::LazyLock;

#[path = "signatures.rs"]
mod signatures;

/// A signature name only; never contains the matched credential or source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretKind(pub &'static str);

static PATTERNS: LazyLock<Vec<(&str, Regex)>> = LazyLock::new(|| {
    signatures::SIGNATURES
        .iter()
        .map(|(name, pattern)| (*name, Regex::new(pattern).expect("static credential regex")))
        .collect()
});

fn normalize(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        if matches!(c, '\u{ad}' | '\u{34f}' | '\u{61c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{fe00}'..='\u{fe0f}' | '\u{feff}')
        {
            continue;
        }
        if ('\u{ff01}'..='\u{ff5e}').contains(&c) {
            normalized.push(char::from_u32(c as u32 - 0xfee0).unwrap());
        } else {
            normalized.push(c);
        }
    }
    normalized
}

/// Apply to decoded input strings and complete, reassembled output before
/// publishing, tracing or persistence. Blocks the whole payload: a signature
/// can be merely a marker for undetectable companion credentials.
/// This is bounded-pattern detection, not arbitrary encoding detection or DLP.
pub fn check_secrets(text: &str) -> Result<(), Error> {
    if let Some(kind) = detect_secret(text) {
        return Err(Error::new(
            ErrorCategory::Guardrail,
            format!("potential {} detected; payload blocked", kind.0),
        ));
    }
    Ok(())
}

/// SDK-compatible diagnostic redaction, not permission to publish an otherwise
/// blocked tool result. Companion credentials may remain outside matched spans.
pub fn redact_secrets(text: &str) -> (String, Vec<SecretKind>, usize) {
    static PEM_END: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"-----END (?:RSA |EC |DSA |OPENSSH |PGP |ENCRYPTED )?PRIVATE KEY-----")
            .expect("static PEM end pattern")
    });
    let mut content = text.to_owned();
    let mut kinds = Vec::new();
    let mut total = 0;
    for (name, pattern) in PATTERNS.iter() {
        let marker = format!("[REDACTED:{name}]");
        let count;
        if *name == "private key" {
            let mut remaining = content.as_str();
            let mut result = String::new();
            let mut matches = 0;
            while let Some(begin) = pattern.find(remaining) {
                matches += 1;
                result.push_str(&remaining[..begin.start()]);
                result.push_str(&marker);
                remaining = &remaining[begin.end()..];
                if let Some(end) = PEM_END.find(remaining) {
                    remaining = &remaining[end.end()..];
                } else {
                    remaining = "";
                    break;
                }
            }
            result.push_str(remaining);
            content = result;
            count = matches;
        } else {
            count = pattern.find_iter(&content).count();
            if count != 0 {
                content = pattern
                    .replace_all(&content, regex::NoExpand(&marker))
                    .into_owned();
            }
        }
        if count != 0 {
            kinds.push(SecretKind(name));
            total += count;
        }
    }
    (content, kinds, total)
}

pub fn detect_secret(text: &str) -> Option<SecretKind> {
    // Scan both views so control-sequence removal cannot hide a raw credential.
    let normalized = normalize(text);
    PATTERNS
        .iter()
        .find(|(_, regex)| regex.is_match(text) || regex.is_match(&normalized))
        .map(|(name, _)| SecretKind(name))
}
