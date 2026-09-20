//! SDK-compatible qualified identifiers. SHA-1 is only a display-name suffix,
//! never a security or approval digest (those use SHA-256).
use sha1::{Digest, Sha1};
use std::collections::BTreeSet;

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

pub(crate) fn ensure_unique_tool_name(
    candidate: &str,
    used: &mut BTreeSet<String>,
) -> Option<String> {
    if used.insert(candidate.to_owned()) {
        return Some(candidate.to_owned());
    }
    for i in 2..1002 {
        let suffix = format!("_{i}");
        let name = format!(
            "{}{}",
            &candidate[..candidate.len().min(64 - suffix.len())],
            suffix
        );
        if used.insert(name.clone()) {
            return Some(name);
        }
    }
    let digest = Sha1::digest(format!("{candidate}_{}", used.len()).as_bytes());
    let hash: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    let name = format!("{}_{hash}", &candidate[..candidate.len().min(55)]);
    used.insert(name.clone()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_collision_suffixes_truncate_and_use_bounded_hash_fallback() {
        let mut used = BTreeSet::new();
        assert_eq!(
            ensure_unique_tool_name("mcp__a__b", &mut used).unwrap(),
            "mcp__a__b"
        );
        for i in 2..1002 {
            assert_eq!(
                ensure_unique_tool_name("mcp__a__b", &mut used).unwrap(),
                format!("mcp__a__b_{i}")
            );
        }
        assert_eq!(
            ensure_unique_tool_name("mcp__a__b", &mut used).unwrap(),
            "mcp__a__b_d031af2f"
        );
        let candidate = qualified_tool_name("s", &"x".repeat(100));
        let mut used = BTreeSet::new();
        assert_eq!(
            ensure_unique_tool_name(&candidate, &mut used).unwrap(),
            candidate
        );
        assert_eq!(
            ensure_unique_tool_name(&candidate, &mut used).unwrap(),
            format!("{}_2", &candidate[..62])
        );
        assert_eq!(
            ensure_unique_tool_name(&candidate, &mut used).unwrap(),
            format!("{}_3", &candidate[..62])
        );
    }

    #[test]
    fn hash_fallback_collision_does_not_overwrite_an_existing_route() {
        let mut used = BTreeSet::from(["mcp__a__b".to_owned()]);
        used.extend((2..1002).map(|i| format!("mcp__a__b_{i}")));
        let digest = Sha1::digest(b"mcp__a__b_1002");
        let hash: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
        used.insert(format!("mcp__a__b_{hash}"));
        assert_eq!(ensure_unique_tool_name("mcp__a__b", &mut used), None);
        assert_eq!(used.len(), 1002);
    }

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
