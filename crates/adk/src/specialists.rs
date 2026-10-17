//! Standalone SDK-compatible specialist tool-list transformations.
//!
//! These helpers select tool handles, not serialized tool objects or execution
//! authority. In particular, access adapters may rename tools here; callers must
//! still apply their execution authorization policy. Empty results are empty
//! `Vec`s: Go's nil versus non-nil empty slice identity is not represented.

#[cfg(feature = "runtime")]
mod delegation;
#[cfg(feature = "runtime")]
pub use delegation::build_delegation_guide;

use adk_core::{AccessMode, Tool};
use std::{collections::BTreeMap, sync::Arc};

/// SDK tool-list selection tier, independent of execution authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolAccessLevel {
    Full,
    ReadOnly,
}

/// Normalize SDK access strings into its two tiers, not workspace authorization.
/// Empty input selects all tools; unknown nonempty input fails closed.
pub fn normalize_tool_access(value: &str) -> ToolAccessLevel {
    let normalized: String = value
        .trim_matches(|c| {
            matches!(
                c,
                '\u{0009}'..='\u{000d}'
                    | '\u{0020}'
                    | '\u{0085}'
                    | '\u{00a0}'
                    | '\u{1680}'
                    | '\u{2000}'..='\u{200a}'
                    | '\u{2028}'
                    | '\u{2029}'
                    | '\u{202f}'
                    | '\u{205f}'
                    | '\u{3000}'
            )
        })
        .chars()
        .map(|c| match c {
            // These are the non-ASCII Go simple-lowercase mappings into ASCII.
            '\u{0130}' => 'i',
            '\u{212a}' => 'k',
            _ => c.to_ascii_lowercase(),
        })
        .collect();
    match normalized.as_str() {
        "" | "full" | "write" | "workspace-write" | "workspace_write" | "execution"
        | "danger-full-access" | "danger_full_access" => ToolAccessLevel::Full,
        _ => ToolAccessLevel::ReadOnly,
    }
}

/// Adapt for read-only access, then filter the resulting declarations.
/// Full access clones handles without consulting adapters. Order and duplicates
/// are preserved; no tool is executed or authorized by this operation.
pub fn filter_tools_by_access(tools: &[Arc<dyn Tool>], access: &str) -> Vec<Arc<dyn Tool>> {
    if normalize_tool_access(access) == ToolAccessLevel::Full {
        return tools.to_vec();
    }
    tools
        .iter()
        .filter_map(|tool| {
            let adapted = tool
                .for_access(AccessMode::ReadOnly)
                .unwrap_or_else(|| Arc::clone(tool));
            adapted.definition().read_only.then_some(adapted)
        })
        .collect()
}

/// Return a fresh map of the SDK's exact orchestrator-only tool names.
pub fn default_signal_tool_names() -> BTreeMap<String, bool> {
    ["finish", "present_plan", "AskUserQuestion"]
        .into_iter()
        .map(|name| (name.to_owned(), true))
        .collect()
}

/// Remove the default orchestrator-only signals, preserving remaining handles.
pub fn strip_signal_tools(tools: &[Arc<dyn Tool>]) -> Vec<Arc<dyn Tool>> {
    strip_named_signal_tools(tools, &default_signal_tool_names())
}

/// Remove only exact names whose map value is true; false or absent keeps a tool.
pub fn strip_named_signal_tools(
    tools: &[Arc<dyn Tool>],
    signal_tool_names: &BTreeMap<String, bool>,
) -> Vec<Arc<dyn Tool>> {
    tools
        .iter()
        .filter(|tool| signal_tool_names.get(&tool.definition().name) != Some(&true))
        .cloned()
        .collect()
}
