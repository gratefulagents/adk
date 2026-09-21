use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub(crate) fn null_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(
    d: D,
) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}
fn serialize_go_float<S: serde::Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
    if !v.is_finite() {
        return Err(serde::ser::Error::custom("nonfinite Go float"));
    }
    let abs = v.abs();
    let text = if abs != 0.0 && !(1e-6..1e21).contains(&abs) {
        let scientific = format!("{v:e}");
        let (mantissa, exponent) = scientific.split_once('e').expect("scientific format");
        if exponent.starts_with('-') {
            scientific
        } else {
            format!("{mantissa}e+{exponent}")
        }
    } else {
        v.to_string()
    };
    let number: serde_json::Number = text.parse().map_err(serde::ser::Error::custom)?;
    number.serialize(s)
}

fn is_zero<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

// RawMessage distinguishes a nil slice from the explicit JSON bytes `null`.
#[derive(Clone, Debug, Default)]
pub enum RawJson {
    #[default]
    Missing,
    Present(Value),
    Encoded(Box<serde_json::value::RawValue>),
}
impl RawJson {
    pub fn value(&self) -> Option<std::borrow::Cow<'_, Value>> {
        match self {
            Self::Missing => None,
            Self::Present(value) => Some(std::borrow::Cow::Borrowed(value)),
            Self::Encoded(raw) => Some(std::borrow::Cow::Owned(
                serde_json::from_str(raw.get()).expect("validated JSON"),
            )),
        }
    }
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}
impl PartialEq for RawJson {
    fn eq(&self, other: &Self) -> bool {
        self.value() == other.value()
    }
}
impl Serialize for RawJson {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Missing => s.serialize_none(),
            Self::Present(v) => v.serialize(s),
            Self::Encoded(raw) => raw.serialize(s),
        }
    }
}
impl<'de> Deserialize<'de> for RawJson {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Box::<serde_json::value::RawValue>::deserialize(d).map(Self::Encoded)
    }
}
impl JsonSchema for RawJson {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "RawJson".into()
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        Value::json_schema(g)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct RunItemType(pub i64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotType {
    #[default]
    Message,
    ToolCall,
    ToolOutput,
    HandoffCall,
    HandoffOutput,
    Reasoning,
    ToolApproval,
    Compaction,
    Unknown,
}
impl From<RunItemType> for SnapshotType {
    fn from(t: RunItemType) -> Self {
        match t.0 {
            0 => Self::Message,
            1 => Self::ToolCall,
            2 => Self::ToolOutput,
            3 => Self::HandoffCall,
            4 => Self::HandoffOutput,
            5 => Self::Reasoning,
            6 => Self::ToolApproval,
            7 => Self::Compaction,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MessageOutput {
    #[serde(rename = "text", deserialize_with = "null_default")]
    pub text: String,
    #[serde(
        rename = "phase",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub phase: String,
    #[serde(
        rename = "images",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub images: Vec<ImageAttachment>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ImageAttachment {
    #[serde(rename = "media_type", deserialize_with = "null_default")]
    pub media_type: String,
    #[serde(rename = "data", deserialize_with = "null_default")]
    pub data: String,
    #[serde(
        rename = "detail",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ToolCallData {
    #[serde(rename = "id", deserialize_with = "null_default")]
    pub id: String,
    #[serde(rename = "name", deserialize_with = "null_default")]
    pub name: String,
    #[serde(rename = "input")]
    pub input: RawJson,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ToolOutputData {
    #[serde(
        rename = "images",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub images: Vec<ImageAttachment>,
    #[serde(rename = "call_id", deserialize_with = "null_default")]
    pub call_id: String,
    #[serde(rename = "content", deserialize_with = "null_default")]
    pub content: String,
    #[serde(
        rename = "is_error",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub is_error: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HandoffCallData {
    #[serde(rename = "from_agent", deserialize_with = "null_default")]
    pub from_agent: String,
    #[serde(rename = "to_agent", deserialize_with = "null_default")]
    pub to_agent: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HandoffOutputData {
    #[serde(rename = "from_agent", deserialize_with = "null_default")]
    pub from_agent: String,
    #[serde(rename = "to_agent", deserialize_with = "null_default")]
    pub to_agent: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReasoningData {
    #[serde(
        rename = "id",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub id: String,
    #[serde(
        rename = "text",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub text: String,
    #[serde(
        rename = "signature",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub signature: String,
    #[serde(
        rename = "redacted_data",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub redacted_data: String,
    #[serde(
        rename = "encrypted_content",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub encrypted_content: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CompactionData {
    #[serde(
        rename = "id",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub id: String,
    #[serde(
        rename = "content",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub content: String,
    #[serde(
        rename = "encrypted_content",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub encrypted_content: String,
    #[serde(
        rename = "created_by",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub created_by: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ToolApprovalData {
    #[serde(rename = "tool_name", deserialize_with = "null_default")]
    pub tool_name: String,
    #[serde(rename = "input")]
    pub input: RawJson,
    #[serde(rename = "call_id", deserialize_with = "null_default")]
    pub call_id: String,
    #[serde(rename = "approved", deserialize_with = "null_default")]
    pub approved: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Usage {
    #[serde(rename = "requests", deserialize_with = "null_default")]
    pub requests: i64,
    #[serde(rename = "input_tokens", deserialize_with = "null_default")]
    pub input_tokens: i64,
    #[serde(rename = "output_tokens", deserialize_with = "null_default")]
    pub output_tokens: i64,
    #[serde(
        rename = "cache_read_tokens",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub cache_read_tokens: i64,
    #[serde(
        rename = "cache_create_tokens",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub cache_create_tokens: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ResponseSnapshot {
    #[serde(
        rename = "items",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub items: Vec<RunItemSnapshot>,
    #[serde(rename = "usage", deserialize_with = "null_default")]
    pub usage: Usage,
    #[serde(rename = "end_turn", skip_serializing_if = "Option::is_none")]
    pub end_turn: Option<bool>,
    #[serde(
        rename = "texts",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub texts: Vec<String>,
    #[serde(
        rename = "reasoning",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub reasoning: Vec<ReasoningSnapshot>,
    #[serde(
        rename = "reasoning_texts",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub reasoning_texts: Vec<String>,
    #[serde(
        rename = "thinking_texts",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub thinking_texts: Vec<String>,
    #[serde(
        rename = "tool_calls",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub tool_calls: Vec<ToolCallSnapshot>,
    #[serde(rename = "raw", skip_serializing_if = "RawJson::is_missing")]
    pub raw: RawJson,
    #[serde(rename = "raw_available", deserialize_with = "null_default")]
    pub raw_available: bool,
    #[serde(
        rename = "raw_error",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub raw_error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RunItemSnapshot {
    #[serde(rename = "type", deserialize_with = "null_default")]
    pub kind: SnapshotType,
    #[serde(
        rename = "agent_name",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub agent_name: String,
    #[serde(
        rename = "message_text",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub message_text: String,
    #[serde(
        rename = "message_phase",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub message_phase: String,
    #[serde(
        rename = "message_images",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub message_images: Vec<ImageAttachment>,
    #[serde(rename = "tool_call", skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<ToolCallSnapshot>,
    #[serde(rename = "tool_output", skip_serializing_if = "Option::is_none")]
    pub tool_output: Option<ToolOutputData>,
    #[serde(rename = "handoff_call", skip_serializing_if = "Option::is_none")]
    pub handoff_call: Option<HandoffCallData>,
    #[serde(rename = "handoff_output", skip_serializing_if = "Option::is_none")]
    pub handoff_output: Option<HandoffOutputData>,
    #[serde(rename = "reasoning", skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningSnapshot>,
    #[serde(
        rename = "reasoning_text",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub reasoning_text: String,
    #[serde(
        rename = "thinking_text",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub thinking_text: String,
    #[serde(rename = "compaction", skip_serializing_if = "Option::is_none")]
    pub compaction: Option<CompactionSnapshot>,
    #[serde(rename = "tool_approval", skip_serializing_if = "Option::is_none")]
    pub tool_approval: Option<ToolApprovalSnapshot>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ToolCallSnapshot {
    #[serde(
        rename = "id",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub id: String,
    #[serde(
        rename = "name",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub name: String,
    #[serde(rename = "input", skip_serializing_if = "RawJson::is_missing")]
    pub input: RawJson,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReasoningSnapshot {
    #[serde(
        rename = "id",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub id: String,
    #[serde(
        rename = "text",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub text: String,
    #[serde(
        rename = "thinking",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub thinking: String,
    #[serde(
        rename = "signature",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub signature: String,
    #[serde(
        rename = "redacted_data",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub redacted_data: String,
    #[serde(
        rename = "encrypted_content",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub encrypted_content: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CompactionSnapshot {
    #[serde(
        rename = "id",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub id: String,
    #[serde(
        rename = "content",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub content: String,
    #[serde(
        rename = "encrypted_content",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub encrypted_content: String,
    #[serde(
        rename = "created_by",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub created_by: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ToolApprovalSnapshot {
    #[serde(
        rename = "tool_name",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub tool_name: String,
    #[serde(rename = "input", skip_serializing_if = "RawJson::is_missing")]
    pub input: RawJson,
    #[serde(
        rename = "call_id",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub call_id: String,
    #[serde(rename = "approved", deserialize_with = "null_default")]
    pub approved: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ContentEvent {
    #[serde(rename = "ts", deserialize_with = "null_default")]
    pub timestamp: crate::timestamp::GoTimestamp,
    #[serde(rename = "type", deserialize_with = "null_default")]
    pub kind: String,
    #[serde(rename = "session", deserialize_with = "null_default")]
    pub session: i32,
    #[serde(
        rename = "message",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub message: String,
    #[serde(
        rename = "tool",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub tool: String,
    #[serde(
        rename = "tool_use_id",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub tool_use_id: String,
    #[serde(
        rename = "parent_call_id",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub parent_call_id: String,
    #[serde(
        rename = "is_error",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub is_error: bool,
    #[serde(
        rename = "agent_name",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub agent_name: String,
    #[serde(
        rename = "input_raw",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub input_raw: String,
    #[serde(
        rename = "output",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub output: String,
    #[serde(
        rename = "tool_duration_ms",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub tool_duration_ms: i64,
    #[serde(
        rename = "phase",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub phase: String,
    #[serde(
        rename = "llm_attempt",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub llm_attempt: i32,
    #[serde(
        rename = "llm_scope",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub llm_scope: String,
    #[serde(
        rename = "attempt_number",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub attempt_number: i32,
    #[serde(
        rename = "attempt_status",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub attempt_status: String,
    #[serde(
        rename = "scope",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub scope: String,
    #[serde(
        rename = "requested_model",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub requested_model: String,
    #[serde(
        rename = "resolved_model",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub resolved_model: String,
    #[serde(
        rename = "canonical_model",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub canonical_model: String,
    #[serde(
        rename = "provider",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub provider: String,
    #[serde(
        rename = "turn",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub turn: i32,
    #[serde(
        rename = "usage_available",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub usage_available: bool,
    #[serde(
        rename = "has_prompt_tokens",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub has_prompt_tokens: bool,
    #[serde(
        rename = "has_completion_tokens",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub has_completion_tokens: bool,
    #[serde(
        rename = "has_total_tokens",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub has_total_tokens: bool,
    #[serde(
        rename = "prompt_tokens",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub prompt_tokens: i64,
    #[serde(
        rename = "completion_tokens",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub completion_tokens: i64,
    #[serde(
        rename = "total_tokens",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub total_tokens: i64,
    #[serde(
        rename = "attempt_latency_ms",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub attempt_latency_ms: i64,
    #[serde(
        rename = "retry_planned",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub retry_planned: bool,
    #[serde(
        rename = "retry_after_ms",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub retry_after_ms: i64,
    #[serde(
        rename = "fallback_planned",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub fallback_planned: bool,
    #[serde(
        rename = "fallback_from_model",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub fallback_from_model: String,
    #[serde(
        rename = "fallback_to_model",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub fallback_to_model: String,
    #[serde(
        rename = "fallback_reason",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub fallback_reason: String,
    #[serde(
        rename = "failure_kind",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub failure_kind: String,
    #[serde(
        rename = "task_id",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub task_id: String,
    #[serde(
        rename = "status",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub status: String,
    #[serde(
        rename = "subagent_type",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub subagent_type: String,
    #[serde(
        rename = "subagent_model",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub subagent_model: String,
    #[serde(
        rename = "subagent_prompt",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub subagent_prompt: String,
    #[serde(
        rename = "subagent_result_text",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub subagent_result_text: String,
    #[serde(
        rename = "subagent_tool_count",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub subagent_tool_count: i32,
    #[serde(
        rename = "subagent_tokens",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub subagent_tokens: i64,
    #[serde(
        rename = "subagent_duration_ms",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub subagent_duration_ms: i64,
    #[serde(
        rename = "subagent_cost_usd",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    #[serde(serialize_with = "serialize_go_float")]
    pub subagent_cost_usd: f64,
    #[serde(
        rename = "subagent_cost_known",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub subagent_cost_known: bool,
    #[serde(
        rename = "subagent_num_turns",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub subagent_num_turns: i32,
    #[serde(
        rename = "subagent_cache_read_tokens",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub subagent_cache_read_tokens: i64,
    #[serde(
        rename = "subagent_cache_create_tokens",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub subagent_cache_create_tokens: i64,
    #[serde(
        rename = "subagent_stop_reason",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub subagent_stop_reason: String,
    #[serde(
        rename = "subagent_depends_on",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub subagent_depends_on: Vec<String>,
    #[serde(
        rename = "subagent_waiting_on",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub subagent_waiting_on: Vec<String>,
    #[serde(
        rename = "subagent_current_step",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub subagent_current_step: String,
    #[serde(
        rename = "subagent_last_tool",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub subagent_last_tool: String,
    #[serde(
        rename = "subagent_files_written",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub subagent_files_written: i64,
    #[serde(
        rename = "subagent_messages_received",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub subagent_messages_received: i64,
    #[serde(
        rename = "subagent_last_parent_message",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub subagent_last_parent_message: String,
    #[serde(
        rename = "step",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub step: String,
    #[serde(
        rename = "model",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub model: String,
    #[serde(
        rename = "permission_mode",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub permission_mode: String,
    #[serde(
        rename = "cwd",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub cwd: String,
    #[serde(
        rename = "max_turns",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub max_turns: i32,
    #[serde(
        rename = "tools",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub tools: Vec<String>,
    #[serde(
        rename = "mcp_servers",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub mcp_servers: Vec<String>,
    #[serde(
        rename = "cost_usd",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    #[serde(serialize_with = "serialize_go_float")]
    pub cost_usd: f64,
    #[serde(
        rename = "cost_known",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub cost_known: bool,
    #[serde(
        rename = "input_tokens",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub input_tokens: i64,
    #[serde(
        rename = "output_tokens",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub output_tokens: i64,
    #[serde(
        rename = "cache_read_input_tokens",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub cache_read_input_tokens: i64,
    #[serde(
        rename = "cache_creation_input_tokens",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub cache_creation_input_tokens: i64,
    #[serde(
        rename = "input_tokens_include_cache",
        deserialize_with = "null_default"
    )]
    pub input_tokens_include_cache: bool,
    #[serde(
        rename = "input_tokens_include_cache_known",
        deserialize_with = "null_default"
    )]
    pub input_tokens_include_cache_known: bool,
    #[serde(
        rename = "num_turns",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub num_turns: i32,
    #[serde(
        rename = "duration_ms",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub duration_ms: i64,
    #[serde(
        rename = "stop_reason",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub stop_reason: String,
    #[serde(
        rename = "tokens_before",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub tokens_before: i32,
    #[serde(
        rename = "tokens_after",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub tokens_after: i32,
    #[serde(
        rename = "hook_name",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub hook_name: String,
    #[serde(
        rename = "decision",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub decision: String,
    #[serde(
        rename = "reason",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AgentRef {
    #[serde(rename = "Name", deserialize_with = "null_default")]
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "PascalCase")]
pub struct RunItem {
    #[serde(rename = "Type", deserialize_with = "null_default")]
    pub kind: RunItemType,
    pub agent: Option<AgentRef>,
    pub message: Option<MessageOutput>,
    pub tool_call: Option<ToolCallData>,
    pub tool_output: Option<ToolOutputData>,
    pub handoff_call: Option<HandoffCallData>,
    pub handoff_output: Option<HandoffOutputData>,
    pub reasoning: Option<ReasoningData>,
    pub compaction: Option<CompactionData>,
    pub tool_approval: Option<ToolApprovalData>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "PascalCase")]
pub struct ModelResponse {
    pub items: Option<Vec<RunItem>>,
    #[serde(deserialize_with = "null_default")]
    pub usage: Usage,
    pub raw: Value,
    #[serde(rename = "CostUSD", deserialize_with = "null_default")]
    #[serde(serialize_with = "serialize_go_float")]
    pub cost_usd: f64,
    #[serde(deserialize_with = "null_default")]
    pub cost_known: bool,
    pub end_turn: Option<bool>,
    #[serde(deserialize_with = "null_default")]
    pub context_tokens: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "PascalCase")]
pub struct ChildToolEvent {
    #[serde(rename = "ParentCallID")]
    pub parent_call_id: String,
    #[serde(rename = "CallID")]
    pub call_id: String,
    pub agent_name: String,
    pub tool: String,
    pub phase: String,
    pub input_raw: String,
    pub output: String,
    pub is_error: bool,
    #[serde(rename = "DurationMS")]
    pub duration_ms: i64,
}
