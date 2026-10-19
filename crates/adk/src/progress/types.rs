use adk_codec::timestamp::GoTimestamp;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelUsageEntry {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub timestamp: GoTimestamp,
    pub event_type: String,
    pub summary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct ProgressSnapshot {
    pub current_step: String,
    pub session_number: i32,
    pub last_activity: String,
    pub agent_count: i32,
    pub tool_call_count: i32,
    #[serde(serialize_with = "adk_codec::dto::serialize_go_float")]
    pub cost_usd: f64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub model_usage: Option<BTreeMap<String, ModelUsageEntry>>,
    pub api_retries: i32,
    pub events: Vec<ProgressEvent>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct AgentMeta {
    pub description: String,
    pub prompt: String,
    pub subagent_type: String,
    pub model: String,
    pub isolation: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookEvent {
    pub tool_name: String,
    pub event_type: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookResult {
    pub decision: String,
    pub reason: String,
}
