//! Bounded, typed codecs for the pinned Go SDK migration baseline.
pub mod approval;
pub mod config;
pub mod dto;
pub mod request_native;
pub mod snapshots;
mod state;
pub mod timestamp;

use dto::*;
use schemars::{JsonSchema, Schema};
use serde_json::Value;

pub const SUPPORTED_OPERATIONS: &[&str] = &[
    "snapshot_items",
    "response_snapshot",
    "child_event",
    "state_ready",
];

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("unsupported SDK replay operation: {0}")]
    UnsupportedOperation(String),
    #[error("invalid Go baseline JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid state replay: {0}")]
    State(String),
}

pub fn schema<T: JsonSchema>() -> Schema {
    schemars::schema_for!(T)
}

pub fn replay(operation: &str, input: &Value) -> Result<Value, CodecError> {
    Ok(match operation {
        "snapshot_items" => {
            let items: Option<Vec<RunItem>> = serde_json::from_value(input.clone())?;
            let output = snapshot_items(items.as_deref().unwrap_or_default());
            if output.is_empty() {
                Value::Null
            } else {
                serde_json::to_value(output)?
            }
        }
        "response_snapshot" => {
            let response: Option<ModelResponse> = serde_json::from_value(input.clone())?;
            serde_json::to_value(response.as_ref().map(response_snapshot))?
        }
        "child_event" => {
            let event: ContentEvent = serde_json::from_value(input.clone())?;
            serde_json::to_value(child_event(&event))?
        }
        "state_ready" => state::ready(input)?,
        _ => return Err(CodecError::UnsupportedOperation(operation.into())),
    })
}

pub fn snapshot_items(items: &[RunItem]) -> Vec<RunItemSnapshot> {
    items
        .iter()
        .map(|item| {
            let mut out = RunItemSnapshot {
                kind: item.kind.into(),
                agent_name: item
                    .agent
                    .as_ref()
                    .map(|a| a.name.clone())
                    .unwrap_or_default(),
                ..Default::default()
            };
            match out.kind {
                SnapshotType::Message => {
                    if let Some(v) = &item.message {
                        out.message_text = v.text.clone();
                        out.message_phase = v.phase.clone();
                        out.message_images = v.images.clone();
                    }
                }
                SnapshotType::ToolCall => {
                    out.tool_call = item.tool_call.as_ref().map(|v| ToolCallSnapshot {
                        id: v.id.clone(),
                        name: v.name.clone(),
                        input: v.input.clone(),
                    })
                }
                SnapshotType::ToolOutput => out.tool_output = item.tool_output.clone(),
                SnapshotType::HandoffCall => out.handoff_call = item.handoff_call.clone(),
                SnapshotType::HandoffOutput => out.handoff_output = item.handoff_output.clone(),
                SnapshotType::Reasoning => {
                    if let Some(v) = &item.reasoning {
                        out.reasoning_text = v.text.clone();
                        out.thinking_text = v.text.clone();
                        out.reasoning = Some(ReasoningSnapshot {
                            id: v.id.clone(),
                            text: v.text.clone(),
                            thinking: v.text.clone(),
                            signature: v.signature.clone(),
                            redacted_data: v.redacted_data.clone(),
                            encrypted_content: v.encrypted_content.clone(),
                        });
                    }
                }
                SnapshotType::Compaction => {
                    out.compaction = item.compaction.as_ref().map(|v| CompactionSnapshot {
                        id: v.id.clone(),
                        content: v.content.clone(),
                        encrypted_content: v.encrypted_content.clone(),
                        created_by: v.created_by.clone(),
                    })
                }
                SnapshotType::ToolApproval => {
                    out.tool_approval = item.tool_approval.as_ref().map(|v| ToolApprovalSnapshot {
                        tool_name: v.tool_name.clone(),
                        input: v.input.clone(),
                        call_id: v.call_id.clone(),
                        approved: v.approved,
                    })
                }
                SnapshotType::Unknown => {}
            }
            out
        })
        .collect()
}

pub fn response_snapshot(response: &ModelResponse) -> ResponseSnapshot {
    let items = snapshot_items(response.items.as_deref().unwrap_or_default());
    let mut out = ResponseSnapshot {
        usage: response.usage.clone(),
        end_turn: response.end_turn,
        raw_available: !response.raw.is_null(),
        raw: if response.raw.is_null() {
            RawJson::Missing
        } else {
            RawJson::Present(response.raw.clone())
        },
        ..Default::default()
    };
    for item in &items {
        if !item.message_text.is_empty() {
            out.texts.push(item.message_text.clone());
        }
        if let Some(v) = &item.reasoning {
            out.reasoning.push(v.clone());
        }
        if !item.reasoning_text.is_empty() {
            out.reasoning_texts.push(item.reasoning_text.clone());
            out.thinking_texts.push(item.thinking_text.clone());
        }
        if let Some(v) = &item.tool_call {
            out.tool_calls.push(v.clone());
        }
    }
    out.items = items;
    out
}

pub fn child_event(event: &ContentEvent) -> Option<ChildToolEvent> {
    if event.parent_call_id.is_empty() || !matches!(event.kind.as_str(), "tool_start" | "tool_end")
    {
        return None;
    }
    let mut out = ChildToolEvent {
        parent_call_id: event.parent_call_id.clone(),
        call_id: event.tool_use_id.clone(),
        agent_name: event.agent_name.clone(),
        tool: event.tool.clone(),
        ..Default::default()
    };
    if event.kind == "tool_end" {
        out.phase = "end".into();
        out.output = event.output.clone();
        out.is_error = event.is_error;
        out.duration_ms = event.tool_duration_ms;
    } else {
        out.phase = "start".into();
        out.input_raw = event.input_raw.clone();
    }
    Some(out)
}
