//! AGPL platform transcript baseline, isolated from the reusable SDK codec.
use adk_codec::dto::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Sdk(#[from] adk_codec::CodecError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("unsupported transcript version: {0}")]
    Version(i64),
    #[error("unknown persisted run item type")]
    UnknownItem,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Transcript<T> {
    pub version: i64,
    pub floor_message_id: i64,
    pub seen_message_id: i64,
    pub self_assistant_message_id: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pending_user_message_id: i64,
    pub items: Vec<T>,
}
fn is_zero(v: &i64) -> bool {
    *v == 0
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct PersistedRunItem {
    #[serde(rename = "type")]
    pub kind: SnapshotType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<MessageOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<ToolCallData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_output: Option<ToolOutputData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handoff_call: Option<HandoffCallData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handoff_output: Option<HandoffOutputData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compaction: Option<CompactionData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_approval: Option<ToolApprovalData>,
}

pub fn replay(operation: &str, input: &Value) -> Result<Value, Error> {
    if operation != "persist_transcript" {
        return Ok(adk_codec::replay(operation, input)?);
    }
    let source: Transcript<RunItemSnapshot> = serde_json::from_value(input.clone())?;
    if source.version != 1 {
        return Err(Error::Version(source.version));
    }
    let mut items = Vec::with_capacity(source.items.len());
    for item in source.items {
        if item.kind == SnapshotType::Unknown {
            return Err(Error::UnknownItem);
        }
        let mut out = PersistedRunItem {
            kind: item.kind,
            agent: if item.agent_name.is_empty() {
                None
            } else {
                Some(item.agent_name)
            },
            ..Default::default()
        };
        match item.kind {
            SnapshotType::Message => {
                out.message = Some(MessageOutput {
                    text: if item.message_text.is_empty() && !item.message_images.is_empty() {
                        "[image attachment omitted from restart snapshot]".into()
                    } else {
                        item.message_text
                    },
                    ..Default::default()
                })
            }
            SnapshotType::ToolCall => {
                out.tool_call = item.tool_call.map(|v| ToolCallData {
                    id: v.id,
                    name: v.name,
                    input: v.input,
                })
            }
            SnapshotType::ToolOutput => out.tool_output = item.tool_output,
            SnapshotType::HandoffCall => out.handoff_call = item.handoff_call,
            SnapshotType::HandoffOutput => out.handoff_output = item.handoff_output,
            SnapshotType::Reasoning => {
                out.reasoning = item.reasoning.map(|v| ReasoningData {
                    id: v.id,
                    text: v.text,
                    signature: v.signature,
                    redacted_data: v.redacted_data,
                    encrypted_content: v.encrypted_content,
                })
            }
            SnapshotType::Compaction => {
                out.compaction = item.compaction.map(|v| CompactionData {
                    id: v.id,
                    content: v.content,
                    encrypted_content: v.encrypted_content,
                    created_by: v.created_by,
                })
            }
            SnapshotType::ToolApproval => {
                out.tool_approval = item.tool_approval.map(|v| ToolApprovalData {
                    tool_name: v.tool_name,
                    input: v.input,
                    call_id: v.call_id,
                    approved: v.approved,
                })
            }
            SnapshotType::Unknown => unreachable!(),
        }
        items.push(out);
    }
    Ok(serde_json::to_value(Transcript {
        version: source.version,
        floor_message_id: source.floor_message_id,
        seen_message_id: source.seen_message_id,
        self_assistant_message_id: source.self_assistant_message_id,
        pending_user_message_id: source.pending_user_message_id,
        items,
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn platform_fixture_transform_and_typed_wire_roundtrip() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../fixtures/platform.json")).unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 1);
        for case in cases {
            let actual = replay(case["operation"].as_str().unwrap(), &case["input"]).unwrap();
            assert_eq!(actual, case["expected"]);
            let wire: Transcript<PersistedRunItem> =
                serde_json::from_value(actual.clone()).unwrap();
            assert_eq!(serde_json::to_value(wire).unwrap(), actual);
        }
    }
    #[test]
    fn rejects_unknown_types_and_strips_message_images() {
        let envelope = |items: Value| serde_json::json!({"version":1,"floor_message_id":0,"seen_message_id":0,"self_assistant_message_id":0,"items":items});
        assert!(
            replay(
                "persist_transcript",
                &envelope(serde_json::json!([{"type":"future"}]))
            )
            .is_err()
        );
        let result = replay("persist_transcript", &envelope(serde_json::json!([{"type":"message","message_images":[{"data":"abc","media_type":"image/png"}]}]))).unwrap();
        assert_eq!(
            result["items"][0]["message"],
            serde_json::json!({"text":"[image attachment omitted from restart snapshot]"})
        );
    }
}
