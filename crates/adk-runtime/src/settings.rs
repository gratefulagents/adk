//! Native mode-routing settings. Unrecognized labels leave settings unchanged.

use serde_json::{Map, Value};

/// Reasoning effort and budget before provider-specific model/token clamping.
/// `none` supplies an effort but no budget; merging it does not erase a prior budget.
pub fn reasoning_settings(level: &str) -> Map<String, Value> {
    // SDK labels use Unicode simple lowercase, not expanding lowercase mappings.
    let level: String = level
        .trim()
        .chars()
        .map(|c| c.to_lowercase().next().unwrap())
        .collect();
    let budget = match level.as_str() {
        "none" => None,
        "minimal" => Some(1024),
        "low" => Some(2048),
        "medium" => Some(4096),
        "high" => Some(8192),
        "xhigh" => Some(16384),
        "max" => Some(24576),
        _ => return Map::new(),
    };
    let mut settings = Map::from_iter([("reasoning_effort".into(), Value::String(level))]);
    if let Some(budget) = budget {
        settings.insert("thinking_budget".into(), Value::from(budget));
    }
    settings
}

pub fn verbosity_settings(level: &str) -> Map<String, Value> {
    let level: String = level
        .trim()
        .chars()
        .map(|c| c.to_lowercase().next().unwrap())
        .collect();
    match level.as_str() {
        "low" | "medium" | "high" => {
            Map::from_iter([("text_verbosity".into(), Value::String(level))])
        }
        _ => Map::new(),
    }
}

pub fn routing_settings(reasoning: &str, verbosity: &str) -> Map<String, Value> {
    let mut settings = reasoning_settings(reasoning);
    settings.extend(verbosity_settings(verbosity));
    settings
}
