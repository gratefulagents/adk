//! Explicit approval phases and ordered Go history markers beside native run items.
use crate::dto;
use adk_core::{ApprovalRequest, Content, Message, Role, RunItem, ToolCall, ToolOutput};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unsupported approval/history conversion: {0}")]
pub struct BridgeError(pub &'static str);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ApprovalPhase {
    Pending,
    Approved,
    Denied,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ApprovalMarker {
    pub data: dto::ToolApprovalData,
    pub phase: ApprovalPhase,
    pub agent: Option<dto::AgentRef>,
}

impl ApprovalMarker {
    pub fn from_call(call: &ToolCall, phase: ApprovalPhase, agent: Option<dto::AgentRef>) -> Self {
        Self {
            data: dto::ToolApprovalData {
                tool_name: call.name.clone(),
                input: dto::RawJson::Present(call.arguments.clone()),
                call_id: call.id.clone(),
                approved: phase == ApprovalPhase::Approved,
            },
            phase,
            agent,
        }
    }

    /// Go markers have no reason field. The caller supplies it explicitly.
    pub fn to_request(&self, reason: impl Into<String>) -> Result<ApprovalRequest, BridgeError> {
        self.validate()?;
        Ok(ApprovalRequest {
            call: approval_call(&self.data)?,
            reason: reason.into(),
        })
    }

    pub fn validate(&self) -> Result<(), BridgeError> {
        if self.data.approved != (self.phase == ApprovalPhase::Approved) {
            return Err(BridgeError("approval phase contradicts Approved"));
        }
        Ok(())
    }

    pub fn to_wire(&self) -> Result<dto::RunItem, BridgeError> {
        self.validate()?;
        Ok(dto::RunItem {
            kind: dto::RunItemType(6),
            agent: self.agent.clone(),
            tool_approval: Some(self.data.clone()),
            ..Default::default()
        })
    }
}

pub fn approval_call(data: &dto::ToolApprovalData) -> Result<ToolCall, BridgeError> {
    Ok(ToolCall {
        id: data.call_id.clone(),
        name: data.tool_name.clone(),
        arguments: data
            .input
            .value()
            .ok_or(BridgeError("missing approval input is not JSON null"))?
            .into_owned(),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalMarkerBoundary {
    /// Insert immediately before this native item; items.len() appends at the end.
    /// Equal boundaries retain marker slice order.
    pub before_item: usize,
    pub marker: ApprovalMarker,
}

/// Provenance and marker phases cannot be recovered from native RunItem alone.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeHistory {
    pub items: Vec<RunItem>,
    pub agents: Vec<Option<dto::AgentRef>>,
    pub markers: Vec<ApprovalMarkerBoundary>,
}

fn encode_content(content: &[Content]) -> Result<(String, Vec<dto::ImageAttachment>), BridgeError> {
    let (text, attachments) = match content.split_first() {
        Some((Content::Text { text }, rest)) => (text.clone(), rest),
        _ => (String::new(), content),
    };
    let images = attachments
        .iter()
        .map(|content| match content {
            Content::Attachment {
                media_type,
                data,
                detail,
            } => Ok(dto::ImageAttachment {
                media_type: media_type.clone(),
                data: data.clone(),
                detail: detail.clone(),
            }),
            _ => Err(BridgeError(
                "wire content requires optional leading text followed by inline attachments",
            )),
        })
        .collect::<Result<_, _>>()?;
    Ok((text, images))
}

fn decode_content(text: &str, images: &[dto::ImageAttachment]) -> Vec<Content> {
    let mut content = vec![Content::Text { text: text.into() }];
    content.extend(images.iter().map(|image| Content::Attachment {
        media_type: image.media_type.clone(),
        data: image.data.clone(),
        detail: image.detail.clone(),
    }));
    content
}

pub fn encode_item(
    item: &RunItem,
    agent: Option<&dto::AgentRef>,
) -> Result<dto::RunItem, BridgeError> {
    let mut wire = dto::RunItem {
        agent: agent.cloned(),
        ..Default::default()
    };
    match item {
        RunItem::Message { message } | RunItem::PhasedMessage { message, .. } => {
            if !matches!(
                (message.role, agent.is_some()),
                (Role::User, false) | (Role::Assistant, true)
            ) {
                return Err(BridgeError(
                    "message role requires matching explicit agent provenance",
                ));
            }
            let phase = match item {
                RunItem::PhasedMessage { phase, .. } if phase.is_empty() => {
                    return Err(BridgeError("empty phase must use an ordinary Message"));
                }
                RunItem::PhasedMessage { phase, .. } => phase.clone(),
                _ => String::new(),
            };
            let (text, images) = encode_content(&message.content)?;
            wire.message = Some(dto::MessageOutput {
                text,
                phase,
                images,
            });
        }
        RunItem::ToolCall { call } => {
            wire.kind = dto::RunItemType(1);
            wire.tool_call = Some(dto::ToolCallData {
                id: call.id.clone(),
                name: call.name.clone(),
                input: dto::RawJson::Present(call.arguments.clone()),
            });
        }
        RunItem::ToolResult { call_id, output } => {
            if output.should_pause {
                return Err(BridgeError("Go ToolOutputData cannot carry should_pause"));
            }
            wire.kind = dto::RunItemType(2);
            let (content, images) = encode_content(&output.content)?;
            wire.tool_output = Some(dto::ToolOutputData {
                call_id: call_id.clone(),
                content,
                images,
                is_error: output.is_error,
            });
        }
        RunItem::Reasoning { reasoning } => {
            wire.kind = dto::RunItemType(5);
            wire.reasoning = Some(dto::ReasoningData {
                id: reasoning.id.clone(),
                text: reasoning.text.clone(),
                signature: reasoning.signature.clone(),
                redacted_data: reasoning.redacted_data.clone(),
                encrypted_content: reasoning.encrypted_content.clone(),
            });
        }
        RunItem::Compaction { compaction } => {
            wire.kind = dto::RunItemType(7);
            wire.compaction = Some(dto::CompactionData {
                id: compaction.id.clone(),
                content: compaction.content.clone(),
                encrypted_content: compaction.encrypted_content.clone(),
                created_by: compaction.created_by.clone(),
            });
        }
        RunItem::Handoff { .. } => {
            return Err(BridgeError(
                "native and Go handoffs have different payloads",
            ));
        }
    }
    Ok(wire)
}

pub fn decode_item(wire: &dto::RunItem) -> Result<RunItem, BridgeError> {
    let item = match wire.kind.0 {
        0 => {
            let message = wire
                .message
                .as_ref()
                .ok_or(BridgeError("missing Message"))?;
            let native = Message {
                role: if wire.agent.is_some() {
                    Role::Assistant
                } else {
                    Role::User
                },
                content: decode_content(&message.text, &message.images),
            };
            if message.phase.is_empty() {
                RunItem::Message { message: native }
            } else {
                RunItem::PhasedMessage {
                    message: native,
                    phase: message.phase.clone(),
                }
            }
        }
        1 => {
            let call = wire
                .tool_call
                .as_ref()
                .ok_or(BridgeError("missing ToolCall"))?;
            let Some(arguments) = call.input.value() else {
                return Err(BridgeError("missing tool input is not JSON null"));
            };
            RunItem::ToolCall {
                call: ToolCall {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: arguments.into_owned(),
                },
            }
        }
        2 => {
            let output = wire
                .tool_output
                .as_ref()
                .ok_or(BridgeError("missing ToolOutput"))?;
            RunItem::ToolResult {
                call_id: output.call_id.clone(),
                output: ToolOutput {
                    content: decode_content(&output.content, &output.images),
                    is_error: output.is_error,
                    should_pause: false,
                },
            }
        }
        5 => {
            let data = wire
                .reasoning
                .as_ref()
                .ok_or(BridgeError("missing Reasoning"))?;
            RunItem::Reasoning {
                reasoning: adk_core::Reasoning {
                    id: data.id.clone(),
                    text: data.text.clone(),
                    signature: data.signature.clone(),
                    redacted_data: data.redacted_data.clone(),
                    encrypted_content: data.encrypted_content.clone(),
                },
            }
        }
        7 => {
            let data = wire
                .compaction
                .as_ref()
                .ok_or(BridgeError("missing Compaction"))?;
            RunItem::Compaction {
                compaction: adk_core::Compaction {
                    id: data.id.clone(),
                    content: data.content.clone(),
                    encrypted_content: data.encrypted_content.clone(),
                    created_by: data.created_by.clone(),
                },
            }
        }
        _ => {
            return Err(BridgeError(
                "item type has no native bridge; approval markers require decode_history",
            ));
        }
    };
    // Re-encoding detects ancillary fields that native items cannot retain.
    if encode_item(&item, wire.agent.as_ref())? != *wire {
        return Err(BridgeError(
            "wire item contains unsupported content or extra payloads",
        ));
    }
    Ok(item)
}

pub fn encode_history(
    items: &[RunItem],
    agents: &[Option<dto::AgentRef>],
    markers: &[ApprovalMarkerBoundary],
) -> Result<Vec<dto::RunItem>, BridgeError> {
    if agents.len() != items.len() {
        return Err(BridgeError(
            "one provenance entry is required per native item",
        ));
    }
    let mut previous = 0;
    for boundary in markers {
        if boundary.before_item < previous || boundary.before_item > items.len() {
            return Err(BridgeError(
                "marker boundaries must be ordered and inside history",
            ));
        }
        boundary.marker.validate()?;
        previous = boundary.before_item;
    }
    let mut wire = Vec::new();
    let mut markers = markers.iter().peekable();
    for (index, (item, agent)) in items.iter().zip(agents).enumerate() {
        while markers.peek().is_some_and(|m| m.before_item == index) {
            wire.push(markers.next().unwrap().marker.to_wire()?);
        }
        wire.push(encode_item(item, agent.as_ref())?);
    }
    for boundary in markers {
        wire.push(boundary.marker.to_wire()?);
    }
    Ok(wire)
}

/// One explicit phase per marker in wire order; never guess pending vs denied.
pub fn decode_history(
    wire: &[dto::RunItem],
    phases: &[ApprovalPhase],
) -> Result<NativeHistory, BridgeError> {
    let mut history = NativeHistory {
        items: Vec::new(),
        agents: Vec::new(),
        markers: Vec::new(),
    };
    let mut phases = phases.iter();
    for item in wire {
        if item.kind.0 == 6 {
            let marker = ApprovalMarker {
                data: item
                    .tool_approval
                    .clone()
                    .ok_or(BridgeError("missing ToolApproval"))?,
                phase: *phases
                    .next()
                    .ok_or(BridgeError("missing explicit approval phase"))?,
                agent: item.agent.clone(),
            };
            if marker.to_wire()? != *item {
                return Err(BridgeError("approval item contains extra payloads"));
            }
            history.markers.push(ApprovalMarkerBoundary {
                before_item: history.items.len(),
                marker,
            });
        } else {
            history.items.push(decode_item(item)?);
            history.agents.push(item.agent.clone());
        }
    }
    if phases.next().is_some() {
        return Err(BridgeError("too many approval phases"));
    }
    Ok(history)
}
