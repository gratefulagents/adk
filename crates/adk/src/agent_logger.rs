//! Owned turn/tool logging with SDK message bodies and native Rust formatting.
use crate::content_logger::{ContentLogger, preview, quote};
use adk_codec::dto::{ContentEvent, RunItem};
use std::{
    fmt::Display,
    io::{self, Write},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LogLevel(pub i64);
impl LogLevel {
    pub const NORMAL: Self = Self(0);
    pub const DEBUG: Self = Self(1);
}

/// Unredacted SDK logger bodies, written to an explicitly owned trusted sink.
///
/// `info`, `debug`, `warn` and `error` take Rust `Display` values, including
/// `format_args!`, rather than interpreting Go printf strings. No process-global
/// destination, prefix or timestamp flags are installed. Share `event_logger()`
/// with content streams to keep event and turn/tool writes on the same sink.
/// Flush explicitly; the last logger/event-logger owner releases the sink.
/// Multi-line calls attempt every line and return the first write error.
///
/// ```
/// use adk::{agent_logger::{AgentLogger, LogLevel}, content_stream::ContentStream};
/// let logger = AgentLogger::new(LogLevel::NORMAL, std::io::sink());
/// let stream = ContentStream::new(std::io::sink());
/// stream.set_logger(Some(logger.event_logger()));
/// logger.turn(1, "assistant", "offline").unwrap();
/// logger.info(format_args!("processed {} items", 3)).unwrap();
/// logger.flush().unwrap();
/// ```
pub struct AgentLogger {
    level: LogLevel,
    events: Arc<ContentLogger>,
}

#[derive(Clone, Debug, Default)]
pub struct ModelResponseSummary<'a> {
    pub model: &'a str,
    pub text_count: i64,
    pub tool_count: i64,
    pub thinking_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub stop_reason: &'a str,
}

impl AgentLogger {
    pub fn new(level: LogLevel, writer: impl Write + Send + 'static) -> Self {
        Self {
            level,
            events: Arc::new(ContentLogger::new(writer)),
        }
    }
    pub fn is_debug(&self) -> bool {
        self.level.0 >= LogLevel::DEBUG.0
    }
    pub fn event_logger(&self) -> Arc<ContentLogger> {
        self.events.clone()
    }
    pub fn event(&self, event: &ContentEvent) -> io::Result<()> {
        self.events.event(event)
    }
    pub fn flush(&self) -> io::Result<()> {
        self.events.flush()
    }

    pub fn turn(&self, turn: i64, agent: &str, model: &str) -> io::Result<()> {
        self.info(format_args!(
            "[turn] ===== TURN {turn} agent={} model={} =====",
            quote(agent.as_bytes()),
            quote(model.as_bytes())
        ))
    }
    pub fn turn_end(&self, turn: i64) -> io::Result<()> {
        self.info(format_args!("[turn] ===== END TURN {turn} PAYLOAD ====="))
    }
    pub fn tools(&self, names: &[String], handoffs: &[String], access: &str) -> io::Result<()> {
        self.lines([
            format!("[turn] tools={} {}", names.len(), format_tool_names(names)).into_bytes(),
            format!(
                "[turn] handoffs={} {}",
                handoffs.len(),
                format_tool_names(handoffs)
            )
            .into_bytes(),
            format!("[turn] tool_access={}", quote(access.as_bytes())).into_bytes(),
        ])
    }
    pub fn instructions(&self, instructions: &str) -> io::Result<()> {
        let header = format!("[turn] instructions_len={}", instructions.len()).into_bytes();
        let bytes = if self.is_debug() {
            instructions.as_bytes()
        } else {
            &[]
        };
        let chunks = bytes.chunks(2000).enumerate().map(|(index, bytes)| {
            let start = index * 2000;
            format!(
                "[turn] instructions[{}:{}]={}",
                start,
                start + bytes.len(),
                quote(bytes)
            )
            .into_bytes()
        });
        self.lines(std::iter::once(header).chain(chunks))
    }

    /// Tool-call/output variants require their matching payload, as in the SDK.
    /// Uses UTF-8 DTO text and validated `RawJson`; arbitrary invalid Go raw
    /// messages or outer JSON whitespace cannot be represented by this API.
    pub fn input_items(&self, items: &[RunItem]) -> io::Result<()> {
        let header = format!("[turn] input_items={}", items.len()).into_bytes();
        let lines = items
            .iter()
            .enumerate()
            .map(|(index, item)| match item.kind.0 {
                0 => {
                    let role = if item.agent.is_some() {
                        "assistant"
                    } else {
                        "user"
                    };
                    let text = item
                        .message
                        .as_ref()
                        .map_or("", |message| message.text.as_str());
                    if self.is_debug() {
                        format!(
                            "[turn] input[{index}] type=message role={role} text={}",
                            quote(&preview(text, 500))
                        )
                        .into_bytes()
                    } else {
                        format!(
                            "[turn] input[{index}] type=message role={role} len={}",
                            text.len()
                        )
                        .into_bytes()
                    }
                }
                1 => {
                    let call = item.tool_call.as_ref().expect("tool-call item payload");
                    let mut line = format!(
                        "[turn] input[{index}] type=tool_call name={} id={}",
                        call.name, call.id
                    )
                    .into_bytes();
                    if self.is_debug() {
                        line.extend_from_slice(b" input=");
                        line.extend(preview(&call.input.text(), 300));
                    }
                    line
                }
                2 => {
                    let output = item.tool_output.as_ref().expect("tool-output item payload");
                    let mut line = format!(
                        "[turn] input[{index}] type=tool_output call_id={} is_error={}",
                        output.call_id, output.is_error
                    )
                    .into_bytes();
                    if self.is_debug() {
                        line.extend_from_slice(b" output=");
                        line.extend(preview(&output.content, 300));
                    } else {
                        line.extend_from_slice(format!(" len={}", output.content.len()).as_bytes());
                    }
                    line
                }
                7 => {
                    let (id, length) = item.compaction.as_ref().map_or(("", 0), |data| {
                        (data.id.as_str(), data.encrypted_content.len())
                    });
                    format!("[turn] input[{index}] type=compaction id={id} encrypted_len={length}")
                        .into_bytes()
                }
                kind => format!("[turn] input[{index}] type={kind}").into_bytes(),
            });
        self.lines(std::iter::once(header).chain(lines))
    }
    pub fn tool_exec(&self, name: &str, call_id: &str, input_len: i64) -> io::Result<()> {
        self.info(format_args!(
            "[tool] exec name={name} id={call_id} input_len={input_len}"
        ))
    }
    pub fn tool_result(
        &self,
        name: &str,
        call_id: &str,
        is_error: bool,
        output: &str,
    ) -> io::Result<()> {
        let mut line =
            format!("[tool] result name={name} id={call_id} is_error={is_error}").into_bytes();
        if self.is_debug() {
            line.extend_from_slice(b" output=");
            line.extend(preview(output, 1000));
        } else {
            line.extend_from_slice(format!(" output_len={}", output.len()).as_bytes());
        }
        self.events.write_body(line)
    }
    pub fn model_response(&self, summary: ModelResponseSummary<'_>) -> io::Result<()> {
        self.info(format_args!(
            "[model] response model={} text={} tools={} thinking={} usage=in:{}/out:{} stop={}",
            summary.model,
            summary.text_count,
            summary.tool_count,
            summary.thinking_count,
            summary.input_tokens,
            summary.output_tokens,
            summary.stop_reason
        ))
    }
    pub fn parallel_tool_calls(&self, names: &[String]) -> io::Result<()> {
        self.info(format_args!(
            "[tool] executing {} in parallel: {}",
            names.len(),
            format_tool_names(names)
        ))
    }
    pub fn auto_continue(&self, count: i64, reason: &str) -> io::Result<()> {
        self.info(format_args!("[auto] continue {count}: {reason}"))
    }
    pub fn debug(&self, message: impl Display) -> io::Result<()> {
        if self.is_debug() {
            self.info(format_args!("[debug] {message}"))
        } else {
            Ok(())
        }
    }
    pub fn info(&self, message: impl Display) -> io::Result<()> {
        self.events.write_body(message.to_string().into_bytes())
    }
    pub fn warn(&self, message: impl Display) -> io::Result<()> {
        self.info(format_args!("WARN: {message}"))
    }
    pub fn error(&self, message: impl Display) -> io::Result<()> {
        self.info(format_args!("ERROR: {message}"))
    }

    fn lines(&self, lines: impl IntoIterator<Item = Vec<u8>>) -> io::Result<()> {
        let mut first_error = Ok(());
        for line in lines {
            let result = self.events.write_body(line);
            if first_error.is_ok() {
                first_error = result;
            }
        }
        first_error
    }
}

pub fn format_tool_names(names: &[String]) -> String {
    format!("[{}]", names.join(" "))
}
