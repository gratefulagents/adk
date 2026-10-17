//! SDK tool-input inspection. These helpers never execute tools.
use super::AgentMeta;
use serde::{Deserialize, de::MapAccess};
use serde_json::value::RawValue;

struct Fields(Vec<(String, Box<RawValue>)>);
impl<'de> Deserialize<'de> for Fields {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Fields;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an SDK tool-input object")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Fields, M::Error> {
                let mut fields = Vec::new();
                while let Some(field) = map.next_entry()? {
                    fields.push(field);
                }
                Ok(Fields(fields))
            }
        }
        d.deserialize_map(Visitor)
    }
}

// Validate the whole input before applying updates, retain duplicate-field order,
// ignore null scalar updates, and continue after type errors (LSP ignores them).
fn strings(input: &[u8], names: &[&str]) -> Option<(Vec<String>, bool)> {
    let raw: Box<RawValue> = adk_codec::userinput::decode(input).ok()?;
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for byte in raw.get().bytes() {
        if escaped {
            escaped = false;
        } else if quoted && byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            quoted = !quoted;
        } else if !quoted {
            match byte {
                b'[' | b'{' => {
                    depth += 1;
                    if depth > 10_000 {
                        return None;
                    }
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    let mut values = vec![String::new(); names.len()];
    if raw.get() == "null" {
        return Some((values, false));
    }
    let Fields(fields) = serde_json::from_str(raw.get()).ok()?;
    let mut invalid = false;
    for (name, value) in fields {
        let folded: String = name
            .chars()
            .map(|c| match c {
                'ſ' => 's',
                'K' => 'k',
                _ => c.to_ascii_lowercase(),
            })
            .collect();
        let Some(index) = names
            .iter()
            .position(|name| name.eq_ignore_ascii_case(&folded))
        else {
            continue;
        };
        if value.get() == "null" {
            continue;
        }
        match serde_json::from_str::<String>(value.get()) {
            Ok(value) => values[index] = value,
            Err(_) => invalid = true,
        }
    }
    Some((values, invalid))
}

fn scalar(input: &[u8], name: &str) -> String {
    match strings(input, &[name]) {
        Some((mut values, false)) => values.remove(0),
        _ => String::new(),
    }
}

pub fn extract_file_path(input: &[u8]) -> String {
    scalar(input, "file_path")
}
pub fn extract_path(input: &[u8]) -> String {
    scalar(input, "path")
}
pub fn extract_grep_pattern(input: &[u8]) -> String {
    scalar(input, "pattern")
}
pub fn extract_glob_pattern(input: &[u8]) -> String {
    scalar(input, "pattern")
}
pub fn extract_bash_command(input: &[u8]) -> String {
    scalar(input, "command")
}
pub use adk_codec::userinput::extract_ask_user_question;

pub fn extract_lsp_operation(input: &[u8]) -> (String, String) {
    let Some((mut values, _)) = strings(input, &["operation", "filePath"]) else {
        return (String::new(), String::new());
    };
    let path = values.pop().unwrap();
    (values.pop().unwrap(), path)
}

pub fn extract_agent_meta(input: &[u8]) -> AgentMeta {
    let Some((values, false)) = strings(
        input,
        &[
            "description",
            "prompt",
            "subagent_type",
            "model",
            "isolation",
        ],
    ) else {
        return AgentMeta {
            description: "unknown".into(),
            ..Default::default()
        };
    };
    let description = if values[0].is_empty() {
        &values[1]
    } else {
        &values[0]
    };
    AgentMeta {
        description: truncate(description, 80),
        prompt: values[1].clone(),
        subagent_type: values[2].clone(),
        model: values[3].clone(),
        isolation: values[4].clone(),
    }
}

/// Trim surrounding whitespace, flatten newlines, and bound Unicode scalar count.
pub fn truncate(text: &str, limit: i64) -> String {
    let text = text.trim().replace('\n', " ");
    if limit <= 0 {
        return text;
    }
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    if text.chars().count() <= limit {
        return text;
    }
    if limit <= 3 {
        return text.chars().take(limit).collect();
    }
    let mut out: String = text.chars().take(limit - 3).collect();
    out.push_str("...");
    out
}

/// Bound UTF-8 bytes without splitting a scalar; nonpositive limits disable it.
pub fn truncate_bytes(text: &str, limit: i64) -> String {
    if limit <= 0 {
        return text.into();
    }
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    if text.len() <= limit {
        return text.into();
    }
    let mut end = if limit <= 3 { limit } else { limit - 3 };
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = text[..end].to_owned();
    if limit > 3 {
        out.push_str("...");
    }
    out
}

/// SDK head/tail truncation, including its conservative marker reservation.
pub fn truncate_middle(text: &str, limit: i64) -> String {
    if limit <= 0 {
        return text.into();
    }
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let chars: Vec<_> = text.chars().collect();
    if chars.len() <= limit {
        return text.into();
    }
    let reserve = "\n…[elided %d chars]…\n".len() + 12;
    if limit <= reserve {
        return chars[..limit].iter().collect();
    }
    let keep = limit - reserve;
    let head = keep * 2 / 3;
    let tail = keep - head;
    let mut out: String = chars[..head].iter().collect();
    out.push_str(&format!("\n…[elided {} chars]…\n", chars.len() - keep));
    out.extend(&chars[chars.len() - tail..]);
    out
}
