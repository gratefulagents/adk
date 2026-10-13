//! Host-owned SDK content-event log bodies, without process-global logging state.
mod printable;

use adk_codec::dto::ContentEvent;
use std::{
    fmt::Write as _,
    io::{self, Write},
    sync::Mutex,
};

/// An explicitly owned writer for the SDK `AgentLogger.Event` message format.
///
/// Emits unredacted event details. Send only to a trusted destination. No global
/// logger, timestamp prefix, stderr writer, or implicit flush is installed.
/// Turn/tool logging and Go printf argument formatting are separate contracts.
/// Dropping the logger releases its writer without flushing it.
///
/// ```
/// use adk::{codec::dto::ContentEvent, content_logger::ContentLogger,
///     content_stream::ContentStream};
/// use std::sync::Arc;
/// let logger = Arc::new(ContentLogger::new(std::io::sink()));
/// let stream = ContentStream::new(std::io::sink());
/// stream.set_logger(Some(logger.clone()));
/// stream.emit(ContentEvent { kind: "session_end".into(),
///     status: "completed".into(), ..Default::default() });
/// stream.flush().unwrap();
/// logger.flush().unwrap();
/// stream.set_logger(None);
/// ```
pub struct ContentLogger {
    writer: Mutex<Box<dyn Write + Send>>,
}

impl ContentLogger {
    pub fn new(writer: impl Write + Send + 'static) -> Self {
        Self {
            writer: Mutex::new(Box::new(writer)),
        }
    }

    /// Write one SDK log body. Stream mirroring ignores writer errors, while
    /// direct callers can observe them. Like Go log.Output, short writes are not retried.
    pub fn event(&self, event: &ContentEvent) -> io::Result<()> {
        let mut body = match event.kind.as_str() {
            "assistant_text" => format!(
                "[event] assistant_text len={} preview={}",
                event.message.len(),
                quote(&preview(&event.message, 200))
            ),
            "assistant_thinking" => {
                format!("[event] assistant_thinking len={}", event.message.len())
            }
            "tool_start" => format!(
                "[event] tool_start tool={} id={} agent={}",
                event.tool, event.tool_use_id, event.agent_name
            ),
            "tool_end" => format!(
                "[event] tool_end tool={} id={} is_error={} agent={}",
                event.tool, event.tool_use_id, event.is_error, event.agent_name
            ),
            "phase_transition" => format!(
                "[event] phase_transition name={} msg={}",
                event.phase,
                quote(event.message.as_bytes())
            ),
            "subagent_status" => format!(
                "[event] subagent_status task={} status={} type={} model={} msg={}",
                event.task_id,
                event.status,
                event.subagent_type,
                event.subagent_model,
                quote(event.message.as_bytes())
            ),
            "session_end" => format!("[event] session_end status={}", event.status),
            "llm_attempt" if !event.output.is_empty() => format!(
                "[event] llm_attempt status={} model={} provider={} reason={} latency_ms={} error={}",
                event.attempt_status,
                event.model,
                event.provider,
                event.failure_kind,
                event.attempt_latency_ms,
                quote(&preview(&event.output, 1000))
            ),
            "llm_attempt" => format!(
                "[event] llm_attempt status={} model={} provider={} latency_ms={}",
                event.attempt_status, event.model, event.provider, event.attempt_latency_ms
            ),
            _ => format!("[event] {}", event.kind),
        };
        if !body.ends_with('\n') {
            body.push('\n');
        }
        self.writer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .write(body.as_bytes())
            .map(|_| ())
    }

    pub fn flush(&self) -> io::Result<()> {
        self.writer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .flush()
    }
}

fn preview(text: &str, limit: usize) -> Vec<u8> {
    if text.len() <= limit {
        return text.as_bytes().to_vec();
    }
    let mut bytes = text.as_bytes()[..limit].to_vec();
    bytes.extend_from_slice("…".as_bytes());
    bytes
}

fn quote(mut bytes: &[u8]) -> String {
    let mut out = String::from("\"");
    while !bytes.is_empty() {
        let (valid, invalid) = match std::str::from_utf8(bytes) {
            Ok(_) => (bytes.len(), 0),
            Err(error) => (
                error.valid_up_to(),
                error
                    .error_len()
                    .unwrap_or(bytes.len() - error.valid_up_to()),
            ),
        };
        for ch in std::str::from_utf8(&bytes[..valid])
            .expect("validated prefix")
            .chars()
        {
            let rune = u32::from(ch);
            let index = printable::PRINTABLE.partition_point(|(_, end)| *end < rune);
            let printable = printable::PRINTABLE
                .get(index)
                .is_some_and(|(start, _)| *start <= rune);
            match ch {
                '"' | '\\' => {
                    out.push('\\');
                    out.push(ch);
                }
                _ if printable => out.push(ch),
                '\x07' => out.push_str("\\a"),
                '\x08' => out.push_str("\\b"),
                '\x0c' => out.push_str("\\f"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\x0b' => out.push_str("\\v"),
                _ if rune < 0x20 || rune == 0x7f => {
                    write!(out, "\\x{rune:02x}").unwrap();
                }
                _ if rune < 0x10000 => {
                    write!(out, "\\u{rune:04x}").unwrap();
                }
                _ => {
                    write!(out, "\\U{rune:08x}").unwrap();
                }
            }
        }
        for byte in &bytes[valid..valid + invalid] {
            write!(out, "\\x{byte:02x}").unwrap();
        }
        bytes = &bytes[valid + invalid..];
    }
    out.push('"');
    out
}
