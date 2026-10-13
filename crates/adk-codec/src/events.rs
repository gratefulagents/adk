//! Formatting for typed SDK content events, separate from native observations.
use crate::dto::ContentEvent;

/// Compact debug rendering matching the pinned SDK helper.
///
/// This is not a redaction boundary. Names, status, reasons and error output may
/// contain sensitive or untrusted text. It does not impose an output size limit.
pub fn compact_content_event(event: &ContentEvent) -> String {
    let mut parts = vec![event.kind.clone()];
    for (prefix, value) in [
        ("agent=", &event.agent_name),
        ("tool=", &event.tool),
        ("status=", &event.status),
    ] {
        if !value.is_empty() {
            parts.push(format!("{prefix}{value}"));
        }
    }
    if event.is_error {
        parts.push("error=true".into());
    }
    if !event.failure_kind.is_empty() {
        parts.push(format!("failure={}", event.failure_kind));
    }
    for value in [&event.reason, &event.message] {
        if !value.is_empty() {
            parts.push(first_line(value).into());
        }
    }
    if event.is_error && !event.output.is_empty() {
        parts.push(first_line(&event.output).into());
    }
    parts.join(" ")
}

fn first_line(text: &str) -> &str {
    let text = text.trim();
    text.split_once('\n').map_or(text, |(first, _)| first)
}
