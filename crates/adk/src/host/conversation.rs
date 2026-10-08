//! Caller-driven conversation context and queue helpers matching the pinned SDK.
use super::{RunBatch, UserMessage, WorkingState};
use adk_codec::dto::ImageAttachment;
use adk_core::{Content, ItemProvenance, Message, Role, RunItem};
use std::collections::{BTreeMap, BTreeSet};

pub const DEFAULT_RECENT_CONVERSATION_LIMIT: i64 = 8;
pub const DEFAULT_CONTEXT_MESSAGE_CHAR_LIMIT: i64 = 1200;
pub const DEFAULT_CONTEXT_SUMMARY_CHAR_LIMIT: i64 = 320;
pub const USER_MESSAGE_MODE_ENQUEUE: &str = "enqueue";
pub const USER_MESSAGE_MODE_IMMEDIATE: &str = "immediate";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConversationMessage {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub images: Vec<ImageAttachment>,
}

pub fn build_conversation_tail(
    messages: &[ConversationMessage],
    state: &WorkingState,
    exclude_message_id: i64,
    limit: i64,
) -> RunBatch {
    let limit = if limit <= 0 {
        DEFAULT_RECENT_CONVERSATION_LIMIT
    } else {
        limit
    } as usize;
    let filtered: Vec<_> = messages
        .iter()
        .filter(|message| {
            !(message.id <= state.history_floor_message_id
                || exclude_message_id > 0 && message.id == exclude_message_id
                || message.content.trim().is_empty() && message.images.is_empty())
        })
        .collect();
    let mut batch = RunBatch::default();
    for message in &filtered[filtered.len().saturating_sub(limit)..] {
        let agent = match message.role.as_str() {
            "assistant" => Some("assistant-summary"),
            "system" => Some("system-summary"),
            _ => None,
        };
        batch.items.push(message_item(
            truncate_context_text(&message.content, DEFAULT_CONTEXT_MESSAGE_CHAR_LIMIT),
            &message.images,
            if agent.is_some() {
                Role::Assistant
            } else {
                Role::User
            },
        ));
        batch.provenance.push(match agent {
            Some(name) => ItemProvenance::Agent { name: name.into() },
            None => ItemProvenance::Unattributed,
        });
    }
    batch
}

pub fn build_working_state_context(state: &WorkingState) -> String {
    let mut lines = Vec::new();
    for (label, value) in [
        ("Current objective", &state.goal),
        ("Mode", &state.current_mode),
        ("Current step", &state.current_step),
        ("Latest user direction", &state.last_user_message),
        ("Latest assistant summary", &state.last_assistant_summary),
    ] {
        if value.is_empty() || (label == "Latest user direction" && value == &state.goal) {
            continue;
        }
        lines.push(format!(
            "{label}: {}",
            if label == "Mode" {
                value.clone()
            } else {
                truncate_context_text(value, DEFAULT_CONTEXT_SUMMARY_CHAR_LIMIT)
            }
        ));
    }
    if !state.recent_turn_summaries.is_empty() {
        let start = state.recent_turn_summaries.len().saturating_sub(4);
        lines.push(format!(
            "Recent progress:\n- {}",
            state.recent_turn_summaries[start..]
                .iter()
                .map(|s| truncate_context_text(s, DEFAULT_CONTEXT_SUMMARY_CHAR_LIMIT))
                .collect::<Vec<_>>()
                .join("\n- ")
        ));
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("## Durable Working State\n{}", lines.join("\n"))
    }
}

pub fn derive_working_state_goal(raw_reply: &str, effective_prompt: &str) -> String {
    let raw_reply = raw_reply.trim();
    let effective_prompt = effective_prompt.trim();
    let lower = raw_reply.to_lowercase();
    if raw_reply.is_empty()
        || (!effective_prompt.is_empty()
            && ["approve", "deny", "request changes", "request_changes"]
                .iter()
                .any(|reply| lower == *reply || lower.starts_with(&format!("{reply}:"))))
    {
        effective_prompt.into()
    } else {
        raw_reply.into()
    }
}

pub fn truncate_context_text(text: &str, max: i64) -> String {
    let normalized = text.replace('\n', " ");
    let normalized = normalized.trim();
    if max <= 0 || normalized.chars().count() <= max as usize {
        normalized.into()
    } else {
        format!(
            "{}...",
            normalized.chars().take(max as usize).collect::<String>()
        )
    }
}

pub fn build_assistant_turn_summary(batch: &RunBatch) -> String {
    let mut assistant_messages = Vec::new();
    let mut successes = Vec::new();
    let mut issues = Vec::new();
    for (index, item) in batch.items.iter().enumerate() {
        let (content, max, bullets) = match item {
            RunItem::Message { message } | RunItem::PhasedMessage { message, .. }
                if matches!(
                    batch.provenance.get(index),
                    Some(ItemProvenance::Agent { .. })
                ) =>
            {
                (&message.content, 220, &mut assistant_messages)
            }
            RunItem::ToolResult { output, .. } => (
                &output.content,
                120,
                if output.is_error {
                    &mut issues
                } else {
                    &mut successes
                },
            ),
            _ => continue,
        };
        if bullets.len() >= 2 {
            continue;
        }
        let text: String = content
            .iter()
            .filter_map(|part| match part {
                Content::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let text = truncate_context_text(&text, max).trim().to_owned();
        if !text.is_empty() && !bullets.contains(&text) {
            bullets.push(text);
        }
    }
    let tools = summarize_turn_tool_calls(&batch.items, 4);
    let mut parts = Vec::new();
    if !assistant_messages.is_empty() {
        parts.push(assistant_messages.join("\n"));
    }
    if !tools.is_empty() {
        parts.push(format!("Tools: {}", tools.join(", ")));
    }
    if !successes.is_empty() && assistant_messages.is_empty() {
        parts.push(format!("Key results: {}", successes.join(" | ")));
    }
    if !issues.is_empty() {
        parts.push(format!("Issues: {}", issues.join(" | ")));
    }
    parts.join("\n").trim().into()
}

pub fn summarize_turn_tool_calls(items: &[RunItem], limit: i64) -> Vec<String> {
    let mut counts = BTreeMap::new();
    for item in items {
        if let RunItem::ToolCall { call } = item {
            *counts.entry(call.name.as_str()).or_insert(0usize) += 1;
        }
    }
    let mut pairs: Vec<_> = counts.into_iter().collect();
    pairs.sort_by(|(name_a, count_a), (name_b, count_b)| {
        count_b.cmp(count_a).then_with(|| name_a.cmp(name_b))
    });
    if limit > 0 {
        pairs.truncate(limit as usize);
    }
    pairs
        .into_iter()
        .map(|(name, count)| format!("{name} x {count}"))
        .collect()
}

/// Returns the selected message, leading safe-skip cursor, and immediate priority flag.
pub fn select_next_user_message<'a>(
    messages: &'a [UserMessage],
    consumed_immediate: &BTreeSet<i64>,
) -> (Option<&'a UserMessage>, i64, bool) {
    let mut skip_cursor = 0;
    for message in messages {
        if (message.content.trim().is_empty() && message.images.is_empty())
            || (message.mode == USER_MESSAGE_MODE_IMMEDIATE
                && consumed_immediate.contains(&message.id))
        {
            skip_cursor = message.id;
        } else {
            break;
        }
    }
    let pending = || {
        messages.iter().filter(|message| {
            !(message.content.trim().is_empty() && message.images.is_empty()
                || message.mode == USER_MESSAGE_MODE_IMMEDIATE
                    && consumed_immediate.contains(&message.id))
        })
    };
    if let Some(message) = pending().find(|message| message.mode == USER_MESSAGE_MODE_IMMEDIATE) {
        (Some(message), skip_cursor, true)
    } else {
        (pending().next(), skip_cursor, false)
    }
}

pub fn collect_immediate_run_items(
    messages: &[UserMessage],
    consumed_immediate: &mut BTreeSet<i64>,
) -> (RunBatch, i64) {
    let mut batch = RunBatch::default();
    let mut last_cursor = 0;
    let mut advancing = true;
    for message in messages {
        let content = message.content.trim();
        if content.is_empty() && message.images.is_empty() {
            if advancing {
                last_cursor = message.id;
            }
            continue;
        }
        if message.mode != USER_MESSAGE_MODE_IMMEDIATE {
            advancing = false;
            continue;
        }
        if consumed_immediate.insert(message.id) {
            batch
                .items
                .push(message_item(content.into(), &message.images, Role::User));
            batch.provenance.push(ItemProvenance::Unattributed);
        }
        if advancing {
            last_cursor = message.id;
        }
    }
    (batch, last_cursor)
}

fn message_item(text: String, images: &[ImageAttachment], role: Role) -> RunItem {
    let mut content = vec![Content::Text { text }];
    content.extend(images.iter().map(|image| Content::Attachment {
        media_type: image.media_type.clone(),
        data: image.data.clone(),
        detail: image.detail.clone(),
    }));
    RunItem::Message {
        message: Message { role, content },
    }
}
