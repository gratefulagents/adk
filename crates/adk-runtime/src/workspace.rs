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
