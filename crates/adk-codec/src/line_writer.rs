//! Owned JSONL buffering and typed SDK host-event dispatch.
use crate::dto::{ChildToolEvent, SubagentStreamEvent};
use crate::events::{compact_content_event_line, parse_content_event_line, subagent_event};
use std::io::{self, Write};

/// The three event kinds produced by the SDK JSONL adapter, not the full runtime event enum.
#[derive(Clone, Debug, PartialEq)]
pub enum LineEvent {
    Log(String),
    ChildTool(Box<ChildToolEvent>),
    Subagent(Box<SubagentStreamEvent>),
}

/// An exclusively owned SDK line writer with an ordered, synchronous host sink.
///
/// Partial bytes survive writes, including split UTF-8 codepoints. An empty write
/// or `flush` dispatches the final partial line; drop does not. Invalid UTF-8 is
/// replaced as in Go's JSON string rendering, not retained as arbitrary Go bytes.
/// Hosts needing shared writes should wrap the writer in their own mutex.
/// This adapter does not redact or cap emitted text.
///
/// ```
/// use adk_codec::line_writer::{LineEvent, LineWriter};
/// use std::io::Write;
/// let mut received = Vec::new();
/// {
///     let mut writer = LineWriter::new(|event| received.push(event));
///     writer.write_all(b"hello\n")?;
/// }
/// assert_eq!(received, vec![LineEvent::Log("hello".into())]);
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct LineWriter<F> {
    buffer: Vec<u8>,
    sink: F,
}

impl<F: FnMut(LineEvent)> LineWriter<F> {
    pub fn new(sink: F) -> Self {
        Self {
            buffer: Vec::new(),
            sink,
        }
    }

    fn emit(&mut self, bytes: &[u8], logs_only: bool) {
        let line = crate::userinput::go_utf8(bytes);
        if line.trim().is_empty() {
            return;
        }
        if !logs_only && let Some(event) = parse_content_event_line(&line) {
            if let Some(event) = subagent_event(&event) {
                (self.sink)(LineEvent::Subagent(Box::new(event)));
                return;
            }
            if let Some(event) = crate::child_event(&event) {
                (self.sink)(LineEvent::ChildTool(Box::new(event)));
                return;
            }
        }
        (self.sink)(LineEvent::Log(compact_content_event_line(&line)));
    }
}

impl<F: FnMut(LineEvent)> Write for LineWriter<F> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        const SCANNER_LIMIT: usize = 16 * 1024 * 1024;
        self.buffer.extend_from_slice(bytes);
        let buffered = std::mem::take(&mut self.buffer);
        let mut remaining = buffered.as_slice();
        let mut lines = Vec::new();
        let mut oversized = false;
        while !remaining.is_empty() {
            let newline = remaining.iter().position(|byte| *byte == b'\n');
            let consumed = newline.map_or(remaining.len(), |index| index + 1);
            if consumed > SCANNER_LIMIT || (newline.is_none() && consumed == SCANNER_LIMIT) {
                oversized = true;
                break;
            }
            let line = &remaining[..newline.unwrap_or(consumed)];
            lines.push(line.strip_suffix(b"\r").unwrap_or(line));
            remaining = &remaining[consumed..];
        }
        if oversized {
            // The SDK scanner failure emits prior tokens as logs, then the entire
            // original buffer as one fallback, even when that repeats its first line.
            lines.push(&buffered);
        } else if bytes.last().is_some_and(|byte| *byte != b'\n')
            && let Some(partial) = lines.pop()
        {
            self.buffer.extend_from_slice(partial);
        }
        for line in lines {
            self.emit(line, oversized);
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.write(&[]).map(|_| ())
    }
}
