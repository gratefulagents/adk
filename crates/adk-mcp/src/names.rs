//! SDK-compatible qualified identifiers. SHA-1 is only a display-name suffix,
//! never a security or approval digest (those use SHA-256).
use sha1::{Digest, Sha1};

fn normalize(raw: &str) -> String {
    let mut name = String::new();
    let mut invalid = false;
    for c in raw.trim().chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
            name.push(c);
            invalid = false;
        } else if !invalid {
            name.push('_');
            invalid = true;
        }
    }
    let name = name.trim_matches('_');
    if name.is_empty() {
        "unnamed".into()
    } else {
        name.into()
    }
}

pub fn qualified_tool_name(server: &str, tool: &str) -> String {
    let server = normalize(server);
    let tool = normalize(tool);
    let base = format!("mcp__{server}__{tool}");
    if base.len() <= 64 {
        return base;
    }
    let digest = Sha1::digest(base.as_bytes());
    let hash: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    let mut server_budget = 24;
    let mut tool_budget = 24;
    if server.len() < server_budget {
        tool_budget += server_budget - server.len();
        server_budget = server.len();
    }
    if tool.len() < tool_budget {
        server_budget += tool_budget - tool.len();
        tool_budget = tool.len();
    }
    format!(
        "mcp__{}__{}_{}",
        &server[..server.len().min(server_budget)],
        &tool[..tool.len().min(tool_budget)],
        hash
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_qualified_names() {
        assert_eq!(
            qualified_tool_name(" my.server ", "a/b"),
            "mcp__my_server__a_b"
        );
        assert_eq!(qualified_tool_name("!!!", "___"), "mcp__unnamed__unnamed");
        let name = qualified_tool_name("s", &"x".repeat(100));
        assert_eq!(name.len(), 64);
        assert_eq!(
            name,
            "mcp__s__xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx_c62e11df"
        );
        let name = qualified_tool_name(&"s".repeat(100), &"x".repeat(100));
        assert_eq!(name.len(), 64);
        assert!(name.starts_with(&format!("mcp__{}__{}", "s".repeat(24), "x".repeat(24))));
    }
}
