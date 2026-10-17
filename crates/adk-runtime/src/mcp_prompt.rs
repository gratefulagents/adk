use unicode_general_category::{GeneralCategory, get_general_category};

fn sanitize(name: &str) -> String {
    let mut out = String::new();
    let mut previous_space = false;
    for ch in name.trim().chars() {
        if ch.is_whitespace() {
            if !previous_space {
                out.push(' ');
                previous_space = true;
            }
        } else if !matches!(
            get_general_category(ch),
            GeneralCategory::Control
                | GeneralCategory::Format
                | GeneralCategory::Unassigned
                | GeneralCategory::PrivateUse
                | GeneralCategory::Surrogate
                | GeneralCategory::SpaceSeparator
                | GeneralCategory::LineSeparator
                | GeneralCategory::ParagraphSeparator
        ) {
            out.push(ch);
            previous_space = false;
        }
    }
    out.trim()
        .chars()
        .take(64)
        .collect::<String>()
        .trim()
        .to_owned()
}

pub(crate) fn context(names: &[String]) -> String {
    let names: Vec<_> = names
        .iter()
        .map(|name| sanitize(name))
        .filter(|name| !name.is_empty())
        .collect();
    if names.is_empty() {
        String::new()
    } else {
        format!(
            "# MCP Servers\n\nConnected MCP servers: {}\n\nMCP tools are prefixed as mcp__<server>__<tool>.",
            names.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn unicode_sanitizer_matches_every_pinned_go_scalar_and_context_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/mcp-prompt/observations.json"
        ))
        .unwrap();
        let observed = &fixture["observations"];
        assert_eq!(observed["unicode_version"], "15.0.0");
        assert_eq!(unicode_general_category::UNICODE_VERSION, (15, 0, 0));
        let mut hash = Sha256::new();
        let mut count = 0;
        for ch in (0..=0x10ffff).filter_map(char::from_u32) {
            let value = sanitize(&format!("a{ch}b"));
            hash.update((value.len() as u32).to_le_bytes());
            hash.update(value.as_bytes());
            count += 1;
        }
        assert_eq!(observed["scalar_count"], count);
        assert_eq!(
            observed["scalar_sha256"].as_str().unwrap(),
            format!("{:x}", hash.finalize())
        );
        for case in observed["cases"].as_array().unwrap() {
            let names: Vec<String> = serde_json::from_value(case["names"].clone()).unwrap();
            assert_eq!(context(&names), case["context"].as_str().unwrap());
        }
    }
}
