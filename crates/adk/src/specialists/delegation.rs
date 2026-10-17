use adk_runtime::AgentConfig;
use std::{collections::BTreeMap, sync::Arc};

/// Format the pinned SDK delegation guide without modifying agents or invoking tools.
/// Specialist keys are sorted and blank keys skipped; handoff order is retained.
pub fn build_delegation_guide(
    agent: &AgentConfig,
    specialists: &BTreeMap<String, Arc<AgentConfig>>,
) -> String {
    let subagents: Vec<_> = specialists
        .iter()
        .filter(|(name, _)| !name.trim().is_empty())
        .map(|(name, specialist)| {
            let description = if specialist.handoff_description.is_empty() {
                "Specialist sub-agent"
            } else {
                &specialist.handoff_description
            };
            format!("- {name}: {description}")
        })
        .collect();
    if subagents.is_empty() && agent.handoffs.is_empty() {
        return String::new();
    }
    let mut guide = String::new();
    if !subagents.is_empty() {
        guide.push_str("Available specialist sub-agents (delegate via the subagent tool, agent_name=<name>):\n");
        guide.push_str(&subagents.join("\n"));
        guide.push_str("\n\n");
        guide.push_str(include_str!("delegation.txt"));
        guide.push_str("\n\n");
    }
    if !agent.handoffs.is_empty() {
        guide.push_str("Handoffs (transfer full control):\n");
        let handoffs: Vec<_> = agent
            .handoffs
            .iter()
            .map(|handoff| {
                let description = if handoff.definition.description.is_empty() {
                    format!("Transfer to {}", handoff.target.name)
                } else {
                    handoff.definition.description.clone()
                };
                format!("- {}: {description}", handoff.definition.name)
            })
            .collect();
        guide.push_str(&handoffs.join("\n"));
        guide.push_str("\n\n");
    }
    guide.push_str("Signal tools: finish\n");
    guide
}
