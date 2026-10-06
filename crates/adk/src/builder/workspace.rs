use adk_core::AccessMode;

/// SDK legacy workspace guidance. This is prompt text, not execution authority;
/// its fixed tool list does not assert that those tools were registered.
pub fn workspace_context(work_dir: &str, access: AccessMode) -> String {
    let (access, tools) = if access == AccessMode::ReadOnly {
        (
            "read-only",
            "read-only Bash, list_files, read_file, glob, grep",
        )
    } else {
        (
            "full (read + write + shell)",
            "Bash, Edit, Write, list_files, read_file, glob, grep",
        )
    };
    format!(
        "<environment>\nWorking directory: {work_dir}\nAll file paths are relative to this directory. Use relative paths (e.g., \"internal/foo.go\") instead of absolute paths.\nCRITICAL: Never use /workspace/... absolute paths in tool calls. Always use relative paths from the working directory. Absolute paths outside this directory will be rejected.\nTool access: {access}\nAvailable tools include: {tools}. Use Bash to run rg, fd, cat, and other CLI tools when available.\n</environment>"
    )
}

pub(super) fn selected_workspace_context(
    work_dir: &str,
    access: AccessMode,
    tools: &[String],
) -> String {
    if work_dir.trim().is_empty() {
        return String::new();
    }
    let access = if access == AccessMode::ReadOnly {
        "read-only"
    } else {
        "full"
    };
    let tools = if tools.is_empty() {
        "none".into()
    } else {
        tools.join(", ")
    };
    format!(
        "<environment>\nWorking directory: {work_dir}\nAll file paths are relative to this directory. Use relative paths instead of absolute paths.\nTool access: {access}\nAvailable tools include: {tools}.\n</environment>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_helpers_match_all_pinned_sdk_observations() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../fixtures/workspace-context/observations.json"
        ))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let names: Vec<String> = serde_json::from_value(case["tools"].clone()).unwrap();
            let work = case["work_dir"].as_str().unwrap();
            let accesses: &[AccessMode] = if case["access"] == "read-only" {
                &[AccessMode::ReadOnly]
            } else {
                &[AccessMode::FullAccess, AccessMode::WorkspaceWrite]
            };
            for access in accesses {
                let actual = if case["strict"].as_bool().unwrap() {
                    selected_workspace_context(work, *access, &names)
                } else {
                    workspace_context(work, *access)
                };
                assert_eq!(actual, case["workspace"].as_str().unwrap(), "{case}");
            }
        }
    }
}
