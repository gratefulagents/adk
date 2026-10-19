use std::collections::BTreeMap;

// Flat ownership avoids recursive traversal and drop at Go's JSON depth limit.
enum Node {
    Literal(Vec<u8>),
    String(String),
    Array(Vec<usize>),
    Object(BTreeMap<String, usize>),
}

fn parse(input: &[u8]) -> Result<Vec<Node>, serde_json::Error> {
    super::validate(input)?;
    let normalized = crate::go_decode::normalize_strings(input);
    let mut nodes = Vec::new();
    let mut parents: Vec<(usize, Option<String>)> = Vec::new();
    let mut cursor = 0;
    // Syntax has already been validated. Scan each token once rather than
    // re-parsing each complete subtree (quadratic for deeply nested inputs).
    while cursor < normalized.len() {
        let first = normalized[cursor];
        if first.is_ascii_whitespace() || matches!(first, b',' | b':') {
            cursor += 1;
            continue;
        }
        if matches!(first, b']' | b'}') {
            parents.pop();
            cursor += 1;
            continue;
        }
        let node = match first {
            b'{' | b'[' => {
                cursor += 1;
                if first == b'{' {
                    Node::Object(BTreeMap::new())
                } else {
                    Node::Array(Vec::new())
                }
            }
            b'"' => {
                let end = crate::payload_decode::string_end(&normalized, cursor);
                let text: String = serde_json::from_slice(&normalized[cursor..end])?;
                cursor = end;
                if let Some((parent, key)) = parents.last_mut() {
                    if matches!(&nodes[*parent], Node::Object(_)) && key.is_none() {
                        *key = Some(text);
                        continue;
                    }
                }
                Node::String(text)
            }
            _ => {
                let start = cursor;
                while cursor < normalized.len()
                    && !normalized[cursor].is_ascii_whitespace()
                    && !matches!(normalized[cursor], b',' | b']' | b'}')
                {
                    cursor += 1;
                }
                let text = &normalized[start..cursor];
                if matches!(first, b't' | b'f' | b'n') {
                    Node::Literal(text.to_vec())
                } else {
                    let number: f64 = serde_json::from_slice(text)?;
                    Node::Literal(crate::snapshots::to_go_json(&number)?)
                }
            }
        };
        let index = nodes.len();
        nodes.push(node);
        if let Some((parent, key)) = parents.last_mut() {
            match &mut nodes[*parent] {
                Node::Array(values) => values.push(index),
                Node::Object(values) => {
                    // All overwritten values are still parsed; overflow cannot
                    // be hidden by a later valid value with the same key.
                    values.insert(key.take().expect("validated object key"), index);
                }
                _ => unreachable!("parent is a container"),
            }
        }
        if matches!(first, b'{' | b'[') {
            parents.push((index, None));
        }
    }
    Ok(nodes)
}

fn encode(nodes: &[Node], redact: &impl Fn(&str) -> String) -> Vec<u8> {
    enum Work<'a> {
        Node(usize),
        Byte(u8),
        Key(&'a str),
    }
    let mut out = Vec::new();
    let mut pending = vec![Work::Node(0)];
    while let Some(work) = pending.pop() {
        match work {
            Work::Byte(byte) => out.push(byte),
            Work::Key(key) => out.extend(crate::snapshots::to_go_json(&key).expect("JSON key")),
            Work::Node(index) => match &nodes[index] {
                Node::Literal(bytes) => out.extend(bytes),
                Node::String(text) => {
                    out.extend(crate::snapshots::to_go_json(&redact(text)).expect("JSON string"))
                }
                Node::Array(values) => {
                    out.push(b'[');
                    pending.push(Work::Byte(b']'));
                    for (index, value) in values.iter().enumerate().rev() {
                        pending.push(Work::Node(*value));
                        if index != 0 {
                            pending.push(Work::Byte(b','));
                        }
                    }
                }
                Node::Object(values) => {
                    out.push(b'{');
                    pending.push(Work::Byte(b'}'));
                    for (index, (key, value)) in values.iter().enumerate().rev() {
                        pending.push(Work::Node(*value));
                        pending.push(Work::Byte(b':'));
                        pending.push(Work::Key(key));
                        if index != 0 {
                            pending.push(Work::Byte(b','));
                        }
                    }
                }
            },
        }
    }
    out
}

pub(crate) fn quote_original_bytes(mut input: &[u8]) -> Vec<u8> {
    let mut out = vec![b'"'];
    while !input.is_empty() {
        let (valid, invalid) = match std::str::from_utf8(input) {
            Ok(_) => (input.len(), 0),
            Err(error) => (
                error.valid_up_to(),
                error
                    .error_len()
                    .unwrap_or(input.len() - error.valid_up_to()),
            ),
        };
        let text = std::str::from_utf8(&input[..valid]).expect("validated UTF-8 prefix");
        let quoted = crate::snapshots::to_go_json(&text).expect("JSON string");
        out.extend_from_slice(&quoted[1..quoted.len() - 1]);
        for _ in 0..invalid {
            out.extend_from_slice(b"\\ufffd");
        }
        input = &input[valid + invalid..];
    }
    out.push(b'"');
    out
}

/// Decode raw JSON with Go's float64, Unicode replacement and depth semantics,
/// redact string values, and encode canonical JSON without recursive ownership.
/// Object keys are sorted and are not passed to `redact`. Invalid JSON (including
/// float64 overflow) becomes a redacted string containing the original text,
/// with one replacement character per invalid UTF-8 byte, as in Go.
///
/// The callback is host-owned and receives UTF-8 text. Unchanged fallback text
/// retains Go's escaped invalid-byte spelling; rewritten text is a native UTF-8
/// string. No detector, credentials or global state is used.
pub fn redact_json(input: &[u8], redact: impl Fn(&str) -> String) -> Vec<u8> {
    match parse(input) {
        Ok(nodes) => encode(&nodes, &redact),
        Err(_) => {
            let text = crate::go_decode::go_utf8(input);
            let redacted = redact(&text);
            if redacted == text {
                quote_original_bytes(input)
            } else {
                crate::snapshots::to_go_json(&redacted).expect("JSON string")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn strings_keys_duplicates_and_floats_are_distinct() {
        let seen = RefCell::new(Vec::new());
        let result = redact_json(br#"{"secret":"discard","z":[9007199254740993,-0,true,null],"secret":"keep","a":{"x":"keep"}}"#, |text| {
            seen.borrow_mut().push(text.to_owned());
            text.replace("keep", "redacted")
        });
        assert_eq!(
            result,
            br#"{"a":{"x":"redacted"},"secret":"redacted","z":[9007199254740992,-0,true,null]}"#
        );
        assert_eq!(*seen.borrow(), ["keep", "keep"]);
    }

    #[test]
    fn invalid_input_falls_back_before_redacting_any_decoded_field() {
        for input in [
            br#"{"x":"keep","n":1e400,"n":1}"#.as_slice(),
            b"\"\xe2\x82\" 1e400",
            b"{",
            b"",
        ] {
            let seen = RefCell::new(Vec::new());
            let result = redact_json(input, |text| {
                seen.borrow_mut().push(text.to_owned());
                text.into()
            });
            assert_eq!(seen.borrow().len(), 1);
            assert_eq!(
                serde_json::from_slice::<String>(&result).unwrap(),
                crate::go_decode::go_utf8(input)
            );
        }
    }

    #[test]
    fn maximum_depth_encodes_and_drops_without_recursive_ownership() {
        for (open, close) in [("[", "]"), ("{\"k\":", "}")] {
            let input = format!("{}\"keep\"{}", open.repeat(10_000), close.repeat(10_000));
            let expected = format!(
                "{}\"redacted\"{}",
                open.repeat(10_000),
                close.repeat(10_000)
            );
            assert_eq!(
                redact_json(input.as_bytes(), |_| "redacted".into()),
                expected.as_bytes()
            );
            let invalid = format!("{open}{input}{close}");
            assert_eq!(
                redact_json(invalid.as_bytes(), |_| "fallback".into()),
                br#""fallback""#
            );
        }
    }
}
