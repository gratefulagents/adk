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

/// Ordered multimodal content; media are never fetched by core.
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
    /// Go-compatible inline image or PDF attachment. `data` is base64, not a URI;
    /// an empty `detail` leaves image detail unspecified.
    Attachment {
        media_type: String,
        data: String,
        detail: String,
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
    Message {
        message: Message,
    },
    /// A message with an explicit, nonempty provider phase (for example commentary).
    /// Ordinary `Message` items have no phase; replay must not invent one.
    PhasedMessage {
        message: Message,
        phase: String,
    },
    ToolCall {
        call: ToolCall,
    },
    ToolResult {
        call_id: String,
        output: ToolOutput,
    },
    Handoff {
        call_id: String,
        agent: String,
    },
    /// Provider continuation state is ordered history, not visible assistant output.
    Reasoning {
        reasoning: Reasoning,
    },
    Compaction {
        compaction: Compaction,
    },
}

/// Lossless reasoning continuation. Opaque fields are forwarded only by adapters
/// supporting their encoding; they must not be substituted with display text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Reasoning {
    pub id: String,
    pub text: String,
    pub signature: String,
    pub redacted_data: String,
    pub encrypted_content: String,
}

/// A provider-issued compacted context window, distinct from a local text summary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Compaction {
    pub id: String,
    pub content: String,
    pub encrypted_content: String,
    pub created_by: String,
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
    pub output_schema_name: String,
    pub output_schema_strict: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailPhase {
    Input,
    Output,
    ToolInput,
    ToolOutput,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GuardrailReport {
    pub phase: GuardrailPhase,
    pub guardrail_name: String,
    pub tool_name: Option<String>,
    pub output: Value,
    pub tripwire_triggered: bool,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guardrails: Vec<GuardrailReport>,
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
