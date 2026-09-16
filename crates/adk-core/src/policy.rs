use std::{collections::BTreeSet, num::NonZeroU32, time::Duration};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Host-selected access; a declaration is not a sandbox implementation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    #[default]
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}

/// Host approval policy cannot bypass a tool's own approval requirement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    #[default]
    RequiredByTool,
    All,
}

/// Explicit outcome of pre-execution tool authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolDecision {
    Deny,
    RequireApproval,
    Allow,
}

/// Exact-name authorization policy. An absent allowlist allows all names;
/// an empty allowlist allows none. Denial always wins over approval.
/// Access enforcement and per-tool timeouts remain executor responsibilities.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolPolicy {
    pub access: AccessMode,
    pub allowed_tools: Option<BTreeSet<String>>,
    pub denied_tools: BTreeSet<String>,
    pub allowed_mutating_tools: BTreeSet<String>,
    pub approval: ApprovalPolicy,
    pub timeout: Option<Duration>,
}

impl ToolPolicy {
    /// Evaluate name/access gates before approval. Never treat prefixes as grants.
    pub fn decision(&self, tool: &crate::ToolDefinition) -> ToolDecision {
        if self.denied_tools.contains(&tool.name)
            || self
                .allowed_tools
                .as_ref()
                .is_some_and(|names| !names.contains(&tool.name))
            || (self.access == AccessMode::ReadOnly
                && !tool.read_only
                && !self.allowed_mutating_tools.contains(&tool.name))
        {
            ToolDecision::Deny
        } else if tool.requires_approval || self.approval == ApprovalPolicy::All {
            ToolDecision::RequireApproval
        } else {
            ToolDecision::Allow
        }
    }
}

/// Model loop behavior after tool execution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolUseBehavior {
    #[default]
    Continue,
    StopAfterTool,
}

/// Explicit invocation limits; no hidden default turn budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunPolicy {
    pub max_turns: NonZeroU32,
    pub tools: ToolPolicy,
    pub tool_use: ToolUseBehavior,
}

impl Default for RunPolicy {
    fn default() -> Self {
        Self {
            max_turns: NonZeroU32::new(100).unwrap(),
            tools: ToolPolicy::default(),
            tool_use: ToolUseBehavior::Continue,
        }
    }
}
