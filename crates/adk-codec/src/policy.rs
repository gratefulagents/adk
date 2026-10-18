//! Legacy SDK policy projections, not native execution authorization.
//! Empty legacy labels enable workspace writes and Git remote writes. Hosts must
//! still apply their independent native permission ceiling before executing tools.

use crate::config::ToolPolicySentinels;

pub type PermissionMode = String;
pub type GitRemoteWrites = String;
pub const PERMISSION_MODE_READ_ONLY: &str = "read-only";
pub const PERMISSION_MODE_WORKSPACE_WRITE: &str = "workspace-write";
pub const PERMISSION_MODE_DANGER_FULL_ACCESS: &str = "danger-full-access";
pub const GIT_REMOTE_WRITES_ENABLED: &str = "enabled";
pub const GIT_REMOTE_WRITES_DISABLED: &str = "disabled";

pub fn normalize_permission_mode(mode: &str) -> &'static str {
    match mode.trim() {
        PERMISSION_MODE_WORKSPACE_WRITE | "" => PERMISSION_MODE_WORKSPACE_WRITE,
        PERMISSION_MODE_DANGER_FULL_ACCESS => PERMISSION_MODE_DANGER_FULL_ACCESS,
        _ => PERMISSION_MODE_READ_ONLY,
    }
}

pub fn normalize_git_remote_writes(value: &str) -> &'static str {
    match value.trim() {
        GIT_REMOTE_WRITES_ENABLED | "" => GIT_REMOTE_WRITES_ENABLED,
        _ => GIT_REMOTE_WRITES_DISABLED,
    }
}

pub fn allows_write_tools(mode: &str) -> bool {
    normalize_permission_mode(mode) != PERMISSION_MODE_READ_ONLY
}

pub fn allows_mcp_tool(mode: &str, read_only_hint: bool) -> bool {
    allows_write_tools(mode) || read_only_hint
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct RuntimePolicy {
    pub permission_mode: PermissionMode,
    pub git_remote_writes: GitRemoteWrites,
    #[serde(rename = "EnableMCP")]
    pub enable_mcp: bool,
    pub enable_hooks: bool,
    pub enable_plugins: bool,
    pub allowed_agent_kinds: Option<Vec<String>>,
    pub allowed_skills: Option<Vec<String>>,
}

impl RuntimePolicy {
    pub fn normalize(&self) -> Self {
        Self {
            permission_mode: normalize_permission_mode(&self.permission_mode).into(),
            git_remote_writes: normalize_git_remote_writes(&self.git_remote_writes).into(),
            ..self.clone()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ToolPolicy {
    pub permission_mode: PermissionMode,
}

impl ToolPolicy {
    pub fn new(runtime_policy: &RuntimePolicy) -> Self {
        Self {
            permission_mode: normalize_permission_mode(&runtime_policy.permission_mode).into(),
        }
    }

    pub fn allows_write_tools(&self) -> bool {
        allows_write_tools(&self.permission_mode)
    }
}

pub fn tool_policy_from_permission_mode(mode: &str) -> Option<ToolPolicySentinels> {
    (normalize_permission_mode(mode) != PERMISSION_MODE_DANGER_FULL_ACCESS).then_some(
        ToolPolicySentinels {
            approval_required: true,
            default_timeout: 0,
        },
    )
}

pub fn tool_access_level_from_permission_mode(mode: &str) -> &'static str {
    if allows_write_tools(mode) {
        "full"
    } else {
        "read-only"
    }
}
