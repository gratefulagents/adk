//! Incremental SSE framing over arbitrary byte boundaries, including split UTF-8
//! and CRLF. This layer deliberately does not mistake EOF for model completion.
use adk_core::{Error, ErrorCategory};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub event: String,
    pub data: String,
}

pub struct Decoder {
    line: Vec<u8>,
    data: String,
    event: String,
    bytes: usize,
    limit: usize,
    after_cr: bool,
    first_line: bool,
    failed: bool,
}
impl Default for Decoder {
    fn default() -> Self {
        Self::new(4 * 1024 * 1024)
    }
}
impl Decoder {
    pub fn new(max_event_bytes: usize) -> Self {
        Self {
            line: Vec::new(),
            data: String::new(),
            event: String::new(),
            bytes: 0,
            limit: max_event_bytes,
            after_cr: false,
            first_line: true,
            failed: false,
        }
    }
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<Event>, Error> {
        if self.failed {
            return Err(protocol("SSE decoder is closed"));
        }
        let result = self.feed_inner(chunk);
        if result.is_err() {
            self.failed = true;
            self.line.clear();
            self.data.clear();
        }
        result
    }
    fn feed_inner(&mut self, chunk: &[u8]) -> Result<Vec<Event>, Error> {
        let mut events = Vec::new();
        for &byte in chunk {
            if self.after_cr {
                self.after_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if byte == b'\r' || byte == b'\n' {
                self.after_cr = byte == b'\r';
                if let Some(event) = self.finish_line()? {
                    events.push(event);
                }
            } else {
                self.bytes = self
                    .bytes
                    .checked_add(1)
                    .ok_or_else(|| protocol("SSE frame exceeds limit"))?;
                if self.bytes > self.limit {
                    return Err(protocol("SSE frame exceeds limit"));
                }
                self.line.push(byte);
            }
        }
        Ok(events)
    }
    fn finish_line(&mut self) -> Result<Option<Event>, Error> {
        let raw = std::mem::take(&mut self.line);
        let mut line =
            std::str::from_utf8(&raw).map_err(|_| protocol("SSE contains invalid UTF-8"))?;
        if self.first_line {
            line = line.strip_prefix('\u{feff}').unwrap_or(line);
            self.first_line = false;
        }
        if line.is_empty() {
            self.bytes = 0;
            let event = std::mem::take(&mut self.event);
            if self.data.is_empty() {
                return Ok(None);
            }
            self.data.pop(); // Remove the final SSE-added newline, not user data.
            return Ok(Some(Event {
                event,
                data: std::mem::take(&mut self.data),
            }));
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => {
                self.data.push_str(value);
                self.data.push('\n');
            }
            "event" => self.event = value.to_owned(),
            _ => {} // comments, id, retry and extension fields
        }
        Ok(None)
    }
    /// An unfinished frame is not delivered at EOF. The provider layer must
    /// separately require its protocol-specific terminal event.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.failed = true;
        if !self.line.is_empty() || !self.data.is_empty() || !self.event.is_empty() {
            return Err(protocol("truncated SSE frame"));
        }
        Ok(())
    }
}
fn protocol(message: &'static str) -> Error {
    Error::new(ErrorCategory::Provider, message)
}
