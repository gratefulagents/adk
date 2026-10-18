//! Explicit-map legacy environment expansion; never reads ambient variables.
use std::collections::BTreeMap;

/// Expand `$name` and `${name}` using only `environment`, without recursion.
/// Bytes preserve Go strings, including invalid UTF-8 in names and values.
/// This helper does not grant variables to native sandbox executions.
pub fn expand_safe(value: &[u8], environment: &BTreeMap<Vec<u8>, Vec<u8>>) -> Vec<u8> {
    let mut output = Vec::new();
    let mut rest = value;
    while let Some(dollar) = rest.iter().position(|byte| *byte == b'$') {
        output.extend_from_slice(&rest[..dollar]);
        rest = &rest[dollar + 1..];
        if rest.is_empty() {
            output.push(b'$');
            break;
        }
        let (name, consumed) = if let Some(braced) = rest.strip_prefix(b"{") {
            match braced.iter().position(|byte| *byte == b'}') {
                Some(end) => (&braced[..end], end + 2),
                None => (&b""[..], 1),
            }
        } else if b"*#$@!?-0123456789".contains(&rest[0]) {
            (&rest[..1], 1)
        } else {
            let end = rest
                .iter()
                .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
                .count();
            (&rest[..end], end)
        };
        if consumed == 0 {
            output.push(b'$');
        } else if let Some(replacement) = environment.get(name).filter(|_| !name.is_empty()) {
            output.extend_from_slice(replacement);
        }
        rest = &rest[consumed..];
    }
    output.extend_from_slice(rest);
    output
}
