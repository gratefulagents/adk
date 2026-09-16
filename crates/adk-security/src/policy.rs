use adk_core::{AccessMode, ApprovalPolicy, ToolPolicy};
use std::collections::BTreeSet;

/// Host-authored restrictions. Never deserialize these from tool arguments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecurityPolicy {
    pub tools: ToolPolicy,
    pub git_remote_writes: bool,
    pub allow_network: bool,
    pub approve_mutations: bool,
}

pub fn clamp_access(requested: AccessMode, ceiling: AccessMode) -> AccessMode {
    match (requested, ceiling) {
        (AccessMode::ReadOnly, _) | (_, AccessMode::ReadOnly) => AccessMode::ReadOnly,
        (AccessMode::WorkspaceWrite, _) | (_, AccessMode::WorkspaceWrite) => {
            AccessMode::WorkspaceWrite
        }
        _ => AccessMode::FullAccess,
    }
}

/// Unknown and empty values fail closed, unlike the SDK's legacy empty default.
pub fn normalize_access(value: &str) -> AccessMode {
    match value.trim() {
        "workspace-write" | "workspace_write" => AccessMode::WorkspaceWrite,
        "danger-full-access" | "full_access" => AccessMode::FullAccess,
        _ => AccessMode::ReadOnly,
    }
}

fn intersection(a: &BTreeSet<String>, b: &BTreeSet<String>) -> BTreeSet<String> {
    a.intersection(b).cloned().collect()
}

impl SecurityPolicy {
    /// Meet of platform/run/tool restrictions; denial always wins.
    pub fn compose(&self, other: &Self) -> Self {
        let a = &self.tools;
        let b = &other.tools;
        let allowed_mutating_tools = match (a.access, b.access) {
            (AccessMode::ReadOnly, AccessMode::ReadOnly) => {
                intersection(&a.allowed_mutating_tools, &b.allowed_mutating_tools)
            }
            (AccessMode::ReadOnly, _) => a.allowed_mutating_tools.clone(),
            (_, AccessMode::ReadOnly) => b.allowed_mutating_tools.clone(),
            _ => intersection(&a.allowed_mutating_tools, &b.allowed_mutating_tools),
        };
        Self {
            tools: ToolPolicy {
                access: clamp_access(a.access, b.access),
                allowed_tools: match (&a.allowed_tools, &b.allowed_tools) {
                    (Some(a), Some(b)) => Some(intersection(a, b)),
                    (a, b) => a.clone().or_else(|| b.clone()),
                },
                denied_tools: a.denied_tools.union(&b.denied_tools).cloned().collect(),
                allowed_mutating_tools,
                approval: if a.approval == ApprovalPolicy::All || b.approval == ApprovalPolicy::All
                {
                    ApprovalPolicy::All
                } else {
                    ApprovalPolicy::RequiredByTool
                },
                timeout: match (a.timeout, b.timeout) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                },
            },
            git_remote_writes: self.git_remote_writes && other.git_remote_writes,
            allow_network: self.allow_network && other.allow_network,
            approve_mutations: self.approve_mutations || other.approve_mutations,
        }
    }

    /// Child requests cannot inherit or reintroduce read-only mutation exceptions.
    pub fn for_child(&self, requested: &Self) -> Self {
        let mut child = self.compose(requested);
        child.tools.allowed_mutating_tools.clear();
        child
    }
}
