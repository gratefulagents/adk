use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The author of a conversational message. Tool responses are separate run items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    Developer,
    User,
    Assistant,
}

/// Ordered multimodal content; media are referenced, never fetched by core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    Text {
        text: String,
    },
    Image {
        uri: String,
        media_type: String,
    },
    Audio {
        uri: String,
        media_type: String,
    },
    File {
        uri: String,
        media_type: String,
    },
    Reasoning {
        text: String,
        signature: Option<String>,
    },
}

/// One message with content order preserved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Content>,
}

/// A provider-issued tool invocation. Preserve `id` when producing its result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// A model-visible tool outcome, distinct from an infrastructure failure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolOutput {
    pub content: Vec<Content>,
    pub is_error: bool,
    pub should_pause: bool,
}

/// Ordered conversation entries, including call/result correlation and handoffs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunItem {
    Message { message: Message },
    ToolCall { call: ToolCall },
    ToolResult { call_id: String, output: ToolOutput },
    Handoff { call_id: String, agent: String },
}

/// Provider token counters. Cache counters may be subsets of input tokens;
/// adapters must supply normalized `context_tokens` rather than consumers summing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub context_tokens: Option<u64>,
}

/// A portable tool declaration, without an executable or an authorization grant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: schemars::Schema,
    pub read_only: bool,
    pub requires_approval: bool,
}

/// Provider-neutral model call with explicit schema and adapter-specific settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelRequest {
    pub model: String,
    pub instructions: String,
    pub input: Vec<RunItem>,
    pub tools: Vec<ToolDefinition>,
    pub output_schema: Option<schemars::Schema>,
    pub settings: serde_json::Map<String, Value>,
}

/// A complete model response. `end_turn: None` is distinct from `Some(false)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelResponse {
    pub items: Vec<RunItem>,
    pub usage: Usage,
    pub end_turn: Option<bool>,
    pub response_id: Option<String>,
    pub metadata: serde_json::Map<String, Value>,
}

/// Ordered model stream events. Exactly one `Complete` must precede clean EOF.
/// Errors are returned by the stream method, never disguised as completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelEvent {
    TextDelta { delta: String },
    ReasoningDelta { delta: String },
    ToolArgumentsDelta { call_id: String, delta: String },
    ItemDone { item: RunItem },
    Complete { response: ModelResponse },
}

/// Why execution returned without an infrastructure error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Completed,
    Paused,
    /// Used only in the partial result attached to a run error.
    Incomplete,
}

/// Input history and explicit loop policy for an agent invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RunRequest {
    pub input: Vec<RunItem>,
    pub policy: crate::RunPolicy,
}

/// A host approval boundary. Approval applies to this exact call, not its name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalRequest {
    pub call: ToolCall,
    pub reason: String,
}

/// A successful or partial invocation snapshot, not a durable checkpoint.
/// Resume from `history`, not `input + new_items`: compaction may rewrite history.
/// Pending approvals must be resolved before replaying their calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RunResult {
    pub status: RunStatus,
    pub final_output: Option<Value>,
    pub new_items: Vec<RunItem>,
    pub history: Vec<RunItem>,
    pub responses: Vec<ModelResponse>,
    pub usage: Usage,
    pub pending_approvals: Vec<ApprovalRequest>,
    pub last_agent: Option<String>,
}

/// Events delivered to a host in emission order. The sink provides backpressure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    Started { agent: String },
    Model { event: ModelEvent },
    ToolStarted { call: ToolCall },
    ToolFinished { call_id: String, output: ToolOutput },
    ApprovalRequired { request: ApprovalRequest },
    Finished { result: RunResult },
    Failed { error: crate::ErrorInfo },
}
