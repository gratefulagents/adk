//! Fallible analysis snapshots from explicitly attributed native requests.
//! This projection is not an executable-history codec: SDK snapshot items have
//! no native message role. Unknown authorship and unrepresentable data are errors.

use crate::{
    approval::{ApprovalMarkerBoundary, BridgeError, encode_content, encode_item},
    dto::{self, RawJson, RunItemSnapshot, SnapshotType},
    snapshots::{ModelSettings, OutputSchema, RequestSnapshot, ToolSnapshot},
};
use adk_core::{ItemProvenance, ModelRequest, RunItem};
use std::time::Duration;

/// Preserve explicit authorship without assigning the current agent to history.
/// Unlike executable-history encoding, analysis messages retain text/phase/media
/// without inferring an agent from their native role.
pub fn analysis_item(
    item: &RunItem,
    provenance: &ItemProvenance,
) -> Result<dto::RunItem, BridgeError> {
    let agent = match provenance {
        ItemProvenance::Unknown => return Err(BridgeError("request item provenance is unknown")),
        ItemProvenance::Unattributed => None,
        ItemProvenance::Agent { name } if !name.trim().is_empty() => {
            Some(dto::AgentRef { name: name.clone() })
        }
        ItemProvenance::Agent { .. } => {
            return Err(BridgeError("request item agent name is empty"));
        }
    };
    match item {
        RunItem::Message { message } | RunItem::PhasedMessage { message, .. } => {
            let (text, images) = encode_content(&message.content)?;
            Ok(dto::RunItem {
                agent,
                message: Some(dto::MessageOutput {
                    text,
                    images,
                    phase: match item {
                        RunItem::PhasedMessage { phase, .. } => phase.clone(),
                        _ => String::new(),
                    },
                }),
                ..Default::default()
            })
        }
        _ => encode_item(item, agent.as_ref()),
    }
}

pub fn snapshot_native_items(
    items: &[RunItem],
    provenance: &[ItemProvenance],
) -> Result<Vec<RunItemSnapshot>, BridgeError> {
    if items.len() != provenance.len() {
        return Err(BridgeError("request item provenance length mismatch"));
    }
    let items = items
        .iter()
        .zip(provenance)
        .map(|(item, agent)| analysis_item(item, agent))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(crate::snapshot_items(&items))
}

impl RequestSnapshot {
    /// Build a snapshot with the actual per-tool defaults, in declaration order.
    /// None denotes a tool with no default timeout (SDK zero), not unknown metadata.
    /// Fractional-second timeouts, unknown provenance and adapter-only settings
    /// cannot be represented by the SDK document and are rejected, not discarded.
    pub fn from_native(
        agent_name: &str,
        request: &ModelRequest,
        tool_timeouts: &[Option<Duration>],
    ) -> Result<Self, BridgeError> {
        Self::from_native_with_approvals(agent_name, request, tool_timeouts, &[])
    }

    /// Include explicit journal markers in native-item boundary order. Markers
    /// participate in the SDK estimate but never acquire the current agent's name.
    pub fn from_native_with_approvals(
        agent_name: &str,
        request: &ModelRequest,
        tool_timeouts: &[Option<Duration>],
        markers: &[ApprovalMarkerBoundary],
    ) -> Result<Self, BridgeError> {
        if tool_timeouts.len() != request.tools.len() {
            return Err(BridgeError("request tool timeout metadata length mismatch"));
        }
        const SETTINGS: &[&str] = &[
            "temperature",
            "max_tokens",
            "top_p",
            "tool_choice",
            "parallel_tool_calls",
            "thinking_budget",
            "reasoning_effort",
            "text_verbosity",
            "stop_sequences",
        ];
        if request
            .settings
            .keys()
            .any(|key| key != "prompt_cache_key" && !SETTINGS.contains(&key.as_str()))
        {
            return Err(BridgeError(
                "native request setting has no SDK representation",
            ));
        }
        // Native routing carries this SDK top-level request field in settings.
        // BuildLLMRequestSnapshot intentionally excludes PromptCacheKey.
        let mut settings_value = request.settings.clone();
        if let Some(key) = settings_value.remove("prompt_cache_key") {
            if !key.is_string() {
                return Err(BridgeError(
                    "native prompt cache key exceeds SDK string type",
                ));
            }
        }
        let settings: ModelSettings =
            serde_json::from_value(serde_json::Value::Object(settings_value))
                .map_err(|_| BridgeError("native request settings exceed SDK types or ranges"))?;
        if settings.temperature.is_some_and(|value| !value.is_finite())
            || settings.top_p.is_some_and(|value| !value.is_finite())
        {
            return Err(BridgeError(
                "native request settings exceed SDK finite float range",
            ));
        }
        let tools = request
            .tools
            .iter()
            .zip(tool_timeouts)
            .map(|(tool, timeout)| {
                let timeout_seconds = match timeout {
                    None => 0,
                    Some(timeout) if timeout.subsec_nanos() == 0 => {
                        i64::try_from(timeout.as_secs())
                            .map_err(|_| BridgeError("tool timeout exceeds SDK signed range"))?
                    }
                    Some(_) => {
                        return Err(BridgeError(
                            "fractional tool timeout has no SDK representation",
                        ));
                    }
                };
                Ok(ToolSnapshot {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    input_schema: RawJson::Present(tool.input_schema.as_value().clone()),
                    read_only: tool.read_only,
                    needs_approval: tool.requires_approval,
                    timeout_seconds,
                })
            })
            .collect::<Result<Vec<_>, BridgeError>>()?;
        let input_items = snapshot_native_items(&request.input, &request.input_provenance)?;
        let input_items = merge_markers(input_items, markers)?;
        let input_token_estimate = estimate_items(&input_items)?;
        let mut overhead = tokens(&request.instructions)?
            .checked_add(8)
            .ok_or(BridgeError("request estimate overflow"))?;
        for tool in &tools {
            for value in [
                tokens(&tool.name)?,
                tokens(&tool.description)?,
                raw_tokens(&tool.input_schema)?,
                32,
            ] {
                overhead = overhead
                    .checked_add(value)
                    .ok_or(BridgeError("request estimate overflow"))?;
            }
        }
        let reserve = if settings.max_tokens > 0 {
            settings.max_tokens
        } else {
            16_384
        }
        .max(settings.thinking_budget);
        overhead = overhead
            .checked_add(reserve)
            .and_then(|n| n.checked_add(8_192))
            .ok_or(BridgeError("request estimate overflow"))?;
        let total_token_estimate = input_token_estimate
            .checked_add(overhead)
            .ok_or(BridgeError("request estimate overflow"))?;
        Ok(Self {
            agent_name: agent_name.into(),
            model: request.model.clone(),
            instructions: request.instructions.clone(),
            input_items,
            tools,
            settings,
            output_schema: request.output_schema.as_ref().map(|schema| OutputSchema {
                name: request.output_schema_name.clone(),
                schema: RawJson::Present(schema.as_value().clone()),
                strict: request.output_schema_strict,
            }),
            input_token_estimate,
            request_overhead_token_estimate: overhead,
            total_token_estimate,
        })
    }
}

fn merge_markers(
    items: Vec<RunItemSnapshot>,
    markers: &[ApprovalMarkerBoundary],
) -> Result<Vec<RunItemSnapshot>, BridgeError> {
    let count = items.len();
    let mut previous = 0;
    for boundary in markers {
        if boundary.before_item > count || boundary.before_item < previous {
            return Err(BridgeError(
                "approval marker outside ordered request history",
            ));
        }
        previous = boundary.before_item;
    }
    let mut merged = Vec::new();
    let mut native = items.into_iter();
    let mut position = 0;
    for boundary in markers {
        while position < boundary.before_item {
            merged.push(native.next().expect("validated item boundary"));
            position += 1;
        }
        let wire = boundary.marker.to_wire()?;
        if wire
            .agent
            .as_ref()
            .is_some_and(|agent| agent.name.trim().is_empty())
        {
            return Err(BridgeError("approval marker agent name is empty"));
        }
        merged.extend(crate::snapshot_items(&[wire]));
    }
    merged.extend(native);
    Ok(merged)
}

fn tokens(text: &str) -> Result<i64, BridgeError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(0);
    }
    i64::try_from(text.chars().count() / 4 + 1)
        .map_err(|_| BridgeError("request estimate overflow"))
}
fn raw_tokens(raw: &RawJson) -> Result<i64, BridgeError> {
    match raw {
        RawJson::Missing => Ok(0),
        _ => tokens(
            &serde_json::to_string(raw)
                .map_err(|_| BridgeError("request JSON is not serializable"))?,
        ),
    }
}
fn estimate_items(items: &[RunItemSnapshot]) -> Result<i64, BridgeError> {
    let mut total: i64 = 0;
    for item in items {
        let values = match item.kind {
            SnapshotType::Message => vec![tokens(&item.message_text)?, 8],
            SnapshotType::ToolCall => match &item.tool_call {
                Some(call) => vec![tokens(&call.name)?, raw_tokens(&call.input)?, 16],
                None => vec![],
            },
            SnapshotType::ToolOutput => match &item.tool_output {
                Some(output) => vec![tokens(&output.content)?, 12],
                None => vec![],
            },
            SnapshotType::Reasoning => match &item.reasoning {
                Some(reasoning) => vec![tokens(&reasoning.text)?, 8],
                None => vec![],
            },
            SnapshotType::Compaction => match &item.compaction {
                Some(compaction) => vec![tokens(&compaction.encrypted_content)?.min(20_000), 8],
                None => vec![],
            },
            SnapshotType::HandoffCall => match &item.handoff_call {
                Some(call) => vec![tokens(&call.from_agent)?, tokens(&call.to_agent)?, 8],
                None => vec![],
            },
            SnapshotType::HandoffOutput => match &item.handoff_output {
                Some(output) => vec![tokens(&output.from_agent)?, tokens(&output.to_agent)?, 8],
                None => vec![],
            },
            SnapshotType::ToolApproval => match &item.tool_approval {
                Some(approval) => vec![
                    tokens(&approval.tool_name)?,
                    raw_tokens(&approval.input)?,
                    8,
                ],
                None => vec![],
            },
            SnapshotType::Unknown => vec![8],
        };
        for value in values {
            total = total
                .checked_add(value)
                .ok_or(BridgeError("request estimate overflow"))?;
        }
    }
    Ok(total)
}
