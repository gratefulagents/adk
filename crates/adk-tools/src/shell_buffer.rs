pub(super) const OMISSION: &[u8] = b"\n[output truncated: middle omitted]\n";
pub(super) struct Buffer {
    head: Vec<u8>,
    tail: Vec<u8>,
    pub total: u64,
    pub cap: usize,
    pub capped: bool,
    lost: bool,
    unread_lost: bool,
    cursor: u64,
    terminal: bool,
}
impl Buffer {
    pub fn new(cap: usize, terminal: bool) -> Self {
        Self {
            head: Vec::new(),
            tail: Vec::new(),
            total: 0,
            cap,
            capped: false,
            lost: false,
            unread_lost: false,
            cursor: 0,
            terminal,
        }
    }
    pub fn append(&mut self, data: &[u8], lost: bool) {
        self.lost |= lost;
        self.unread_lost |= lost;
        self.total = self.total.saturating_add(data.len() as u64);
        if self.terminal {
            append_tail(&mut self.tail, data, self.cap);
            self.capped = self.total > self.cap as u64;
        } else if !self.capped && self.head.len() + data.len() <= self.cap {
            self.head.extend_from_slice(data);
        } else {
            let available = self.cap.saturating_sub(OMISSION.len());
            let head_limit = available / 2;
            let tail_limit = available - head_limit;
            if !self.capped {
                append_tail(&mut self.tail, &self.head, tail_limit);
                self.head.truncate(head_limit);
                let keep = data.len().min(head_limit.saturating_sub(self.head.len()));
                self.head.extend_from_slice(&data[..keep]);
                self.capped = true;
            }
            append_tail(&mut self.tail, data, tail_limit);
        }
    }
    pub fn full(&self) -> String {
        let mut bytes = self.head.clone();
        if self.capped && !self.terminal {
            bytes.extend_from_slice(OMISSION);
        }
        bytes.extend_from_slice(&self.tail);
        let mut out = String::from_utf8_lossy(&bytes).into_owned();
        if out.is_empty() {
            out = "(no output yet)".into();
        }
        if self.capped {
            out += &format!(
                "\n[output truncated: {} total bytes produced, response shows head/tail within the {}-byte cap; the process was NOT terminated. Narrow the output instead of re-running unchanged: filter with grep, use head/tail, or write full output to a file and inspect ranges]",
                self.total, self.cap
            );
        }
        if self.lost {
            out += "\n[some output exceeded the sandbox retention cap and was discarded; total bytes is a lower bound]";
        }
        out
    }
    pub fn consume(&mut self) -> (String, bool) {
        let new = self.total.saturating_sub(self.cursor);
        let source = if self.capped || self.terminal {
            &self.tail
        } else {
            &self.head
        };
        let keep = source.len().min(usize::try_from(new).unwrap_or(usize::MAX));
        let gap = self.unread_lost || new > source.len() as u64;
        self.cursor = self.total;
        self.unread_lost = false;
        (
            String::from_utf8_lossy(&source[source.len() - keep..]).into_owned(),
            gap,
        )
    }
}
fn append_tail(tail: &mut Vec<u8>, data: &[u8], limit: usize) {
    if data.len() >= limit {
        tail.clear();
        tail.extend_from_slice(&data[data.len() - limit..]);
    } else {
        let discard = (tail.len() + data.len()).saturating_sub(limit);
        tail.drain(..discard);
        tail.extend_from_slice(data);
    }
}
pub(super) fn terminal_limit(out: String) -> String {
    let bytes = out.as_bytes();
    if bytes.len() <= 10_000 {
        return out;
    }
    format!(
        "{}\n[... output limited to 10000 bytes; {} interior bytes omitted ...]\n{}",
        String::from_utf8_lossy(&bytes[..5000]),
        bytes.len() - 10_000,
        String::from_utf8_lossy(&bytes[bytes.len() - 5000..])
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_head_tail_and_incremental_cursor() {
        let mut b = Buffer::new(100, false);
        b.append(b"first", false);
        assert_eq!(b.consume(), ("first".into(), false));
        b.append(&[b'x'; 200], false);
        b.append(b"last", false);
        assert!(b.full().starts_with("first"));
        assert!(b.full().contains("last"));
        let (out, gap) = b.consume();
        assert!(gap && out.ends_with("last"));
        assert_eq!(b.consume(), (String::new(), false));
        b.append(b"next", false);
        assert_eq!(b.consume(), ("next".into(), false));
        assert!(b.head.len() + b.tail.len() + OMISSION.len() <= 100);
    }
    #[test]
    fn terminal_window_and_return_cap() {
        let mut b = Buffer::new(256 * 1024, true);
        b.append(&vec![b'x'; 300 * 1024], false);
        let (out, gap) = b.consume();
        assert!(gap);
        assert_eq!(out.len(), 256 * 1024);
        assert!(terminal_limit(out).contains("interior bytes omitted"));
    }
}
