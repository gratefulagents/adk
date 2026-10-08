use adk_core::{Error, ErrorCategory};
use regex::Regex;
use std::sync::LazyLock;

#[path = "signatures.rs"]
mod signatures;

/// A signature name only; never contains the matched credential or source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretKind(pub &'static str);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolOutputDisposition {
    Unchanged,
    Blocked { kind: SecretKind },
    Redacted { content: String, notice: String },
}

/// Unlike diagnostic redaction, partial credential markers block the entire output.
/// Obfuscated matches use the same normalized view as input detection.
pub fn sanitize_tool_output(text: &str) -> ToolOutputDisposition {
    let normalized = normalize(text);
    for (name, pattern) in PATTERNS.iter() {
        if matches!(*name, "AWS access key" | "GCP service-account key")
            && (pattern.is_match(text) || pattern.is_match(&normalized))
        {
            return ToolOutputDisposition::Blocked {
                kind: SecretKind(name),
            };
        }
    }
    // Redact both views: normalization can erase raw matches or reveal hidden ones.
    let (raw_redacted, raw_kinds, raw_count) = redact_secrets(text);
    let (content, normalized_kinds, normalized_count) = redact_secrets(&normalize(&raw_redacted));
    let count = raw_count + normalized_count;
    if count == 0 {
        return ToolOutputDisposition::Unchanged;
    }
    let kinds: Vec<_> = PATTERNS
        .iter()
        .filter_map(|(name, _)| {
            (raw_kinds.contains(&SecretKind(name)) || normalized_kinds.contains(&SecretKind(name)))
                .then_some(*name)
        })
        .collect();
    let notice = format!(
        "[guardrail detect-secret-in-output: redacted {count} potential secret(s): {}. If these are placeholders or test fixtures the surrounding content is still usable; do not try to reconstruct redacted values.]",
        kinds.join(", ")
    );
    ToolOutputDisposition::Redacted {
        content: format!("{content}\n\n{notice}"),
        notice,
    }
}

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
