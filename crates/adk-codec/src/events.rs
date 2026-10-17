//! Formatting for typed SDK content events, separate from native observations.
use crate::{dto::ContentEvent, go_decode::IntegerSeed};

/// Compact debug rendering matching the pinned SDK helper.
///
/// This is not a redaction boundary. Names, status, reasons and error output may
/// contain sensitive or untrusted text. It does not impose an output size limit.
pub fn compact_content_event(event: &ContentEvent) -> String {
    let mut parts = vec![event.kind.clone()];
    for (prefix, value) in [
        ("agent=", &event.agent_name),
        ("tool=", &event.tool),
        ("status=", &event.status),
    ] {
        if !value.is_empty() {
            parts.push(format!("{prefix}{value}"));
        }
    }
    if event.is_error {
        parts.push("error=true".into());
    }
    if !event.failure_kind.is_empty() {
        parts.push(format!("failure={}", event.failure_kind));
    }
    for value in [&event.reason, &event.message] {
        if !value.is_empty() {
            parts.push(first_line(value).into());
        }
    }
    if event.is_error && !event.output.is_empty() {
        parts.push(first_line(&event.output).into());
    }
    parts.join(" ")
}

fn first_line(text: &str) -> &str {
    let text = text.trim();
    text.split_once('\n').map_or(text, |(first, _)| first)
}

/// Parse one SDK JSONL event. Malformed records and records without a type return `None`.
///
/// Unlike DTO deserialization, this preserves Go's duplicate-field, null and
/// case-insensitive field matching semantics. Input is not a redaction boundary.
pub fn parse_content_event_line(line: &str) -> Option<ContentEvent> {
    let mut depth = 0_u32;
    let mut inside = false;
    let mut escaped = false;
    for byte in line.bytes() {
        if inside {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                inside = false;
            }
        } else {
            match byte {
                b'"' => inside = true,
                b'[' | b'{' => {
                    depth += 1;
                    if depth > 10_000 {
                        return None;
                    }
                }
                b']' | b'}' => depth = depth.checked_sub(1)?,
                _ => {}
            }
        }
    }
    let parsed = crate::userinput::decode::<ParsedEvent>(line.as_bytes()).ok()?;
    (!parsed.0.kind.is_empty()).then_some(parsed.0)
}

/// Compact an SDK JSONL record, falling back to the first trimmed raw line.
/// Like [`compact_content_event`], this neither redacts nor caps output.
pub fn compact_content_event_line(line: &str) -> String {
    parse_content_event_line(line)
        .as_ref()
        .map(compact_content_event)
        .unwrap_or_else(|| first_line(line).into())
}

struct ParsedEvent(ContentEvent);

impl<'de> serde::Deserialize<'de> for ParsedEvent {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct EventVisitor;
        impl<'de> serde::de::Visitor<'de> for EventVisitor {
            type Value = ParsedEvent;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an SDK content event")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(ParsedEvent(ContentEvent::default()))
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                use crate::userinput::{ElementSeed, GoSlice};
                let mut out = ContentEvent::default();
                let mut tools = GoSlice::<String>::default();
                let mut mcp_servers = GoSlice::<String>::default();
                let mut subagent_depends_on = GoSlice::<String>::default();
                let mut subagent_waiting_on = GoSlice::<String>::default();
                while let Some(key) = map.next_key::<String>()? {
                    let key: String = key
                        .chars()
                        .map(|c| match c {
                            'K' => 'k',
                            'ſ' => 's',
                            _ => c.to_ascii_lowercase(),
                        })
                        .collect();
                    match key.as_str() {
                        "ts" => {
                            if let Some(value) =
                                map.next_value::<Option<Box<serde_json::value::RawValue>>>()?
                            {
                                out.timestamp = crate::timestamp::GoTimestamp::parse_event(
                                    value
                                        .get()
                                        .strip_prefix('"')
                                        .and_then(|v| v.strip_suffix('"'))
                                        .ok_or_else(|| {
                                            serde::de::Error::custom("invalid SDK timestamp")
                                        })?,
                                )
                                .ok_or_else(|| serde::de::Error::custom("invalid SDK timestamp"))?;
                            }
                        }
                        "type" => {
                            map.next_value_seed(ElementSeed(&mut out.kind))?;
                        }
                        "session" => {
                            map.next_value_seed(IntegerSeed(&mut out.session))?;
                        }
                        "message" => {
                            map.next_value_seed(ElementSeed(&mut out.message))?;
                        }
                        "tool" => {
                            map.next_value_seed(ElementSeed(&mut out.tool))?;
                        }
                        "tool_use_id" => {
                            map.next_value_seed(ElementSeed(&mut out.tool_use_id))?;
                        }
                        "parent_call_id" => {
                            map.next_value_seed(ElementSeed(&mut out.parent_call_id))?;
                        }
                        "is_error" => {
                            map.next_value_seed(ElementSeed(&mut out.is_error))?;
                        }
                        "agent_name" => {
                            map.next_value_seed(ElementSeed(&mut out.agent_name))?;
                        }
                        "input_raw" => {
                            map.next_value_seed(ElementSeed(&mut out.input_raw))?;
                        }
                        "output" => {
                            map.next_value_seed(ElementSeed(&mut out.output))?;
                        }
                        "tool_duration_ms" => {
                            map.next_value_seed(IntegerSeed(&mut out.tool_duration_ms))?;
                        }
                        "phase" => {
                            map.next_value_seed(ElementSeed(&mut out.phase))?;
                        }
                        "llm_attempt" => {
                            map.next_value_seed(IntegerSeed(&mut out.llm_attempt))?;
                        }
                        "llm_scope" => {
                            map.next_value_seed(ElementSeed(&mut out.llm_scope))?;
                        }
                        "attempt_number" => {
                            map.next_value_seed(IntegerSeed(&mut out.attempt_number))?;
                        }
                        "attempt_status" => {
                            map.next_value_seed(ElementSeed(&mut out.attempt_status))?;
                        }
                        "scope" => {
                            map.next_value_seed(ElementSeed(&mut out.scope))?;
                        }
                        "requested_model" => {
                            map.next_value_seed(ElementSeed(&mut out.requested_model))?;
                        }
                        "resolved_model" => {
                            map.next_value_seed(ElementSeed(&mut out.resolved_model))?;
                        }
                        "canonical_model" => {
                            map.next_value_seed(ElementSeed(&mut out.canonical_model))?;
                        }
                        "provider" => {
                            map.next_value_seed(ElementSeed(&mut out.provider))?;
                        }
                        "turn" => {
                            map.next_value_seed(IntegerSeed(&mut out.turn))?;
                        }
                        "usage_available" => {
                            map.next_value_seed(ElementSeed(&mut out.usage_available))?;
                        }
                        "has_prompt_tokens" => {
                            map.next_value_seed(ElementSeed(&mut out.has_prompt_tokens))?;
                        }
                        "has_completion_tokens" => {
                            map.next_value_seed(ElementSeed(&mut out.has_completion_tokens))?;
                        }
                        "has_total_tokens" => {
                            map.next_value_seed(ElementSeed(&mut out.has_total_tokens))?;
                        }
                        "prompt_tokens" => {
                            map.next_value_seed(IntegerSeed(&mut out.prompt_tokens))?;
                        }
                        "completion_tokens" => {
                            map.next_value_seed(IntegerSeed(&mut out.completion_tokens))?;
                        }
                        "total_tokens" => {
                            map.next_value_seed(IntegerSeed(&mut out.total_tokens))?;
                        }
                        "attempt_latency_ms" => {
                            map.next_value_seed(IntegerSeed(&mut out.attempt_latency_ms))?;
                        }
                        "retry_planned" => {
                            map.next_value_seed(ElementSeed(&mut out.retry_planned))?;
                        }
                        "retry_after_ms" => {
                            map.next_value_seed(IntegerSeed(&mut out.retry_after_ms))?;
                        }
                        "fallback_planned" => {
                            map.next_value_seed(ElementSeed(&mut out.fallback_planned))?;
                        }
                        "fallback_from_model" => {
                            map.next_value_seed(ElementSeed(&mut out.fallback_from_model))?;
                        }
                        "fallback_to_model" => {
                            map.next_value_seed(ElementSeed(&mut out.fallback_to_model))?;
                        }
                        "fallback_reason" => {
                            map.next_value_seed(ElementSeed(&mut out.fallback_reason))?;
                        }
                        "failure_kind" => {
                            map.next_value_seed(ElementSeed(&mut out.failure_kind))?;
                        }
                        "task_id" => {
                            map.next_value_seed(ElementSeed(&mut out.task_id))?;
                        }
                        "status" => {
                            map.next_value_seed(ElementSeed(&mut out.status))?;
                        }
                        "subagent_type" => {
                            map.next_value_seed(ElementSeed(&mut out.subagent_type))?;
                        }
                        "subagent_model" => {
                            map.next_value_seed(ElementSeed(&mut out.subagent_model))?;
                        }
                        "subagent_prompt" => {
                            map.next_value_seed(ElementSeed(&mut out.subagent_prompt))?;
                        }
                        "subagent_result_text" => {
                            map.next_value_seed(ElementSeed(&mut out.subagent_result_text))?;
                        }
                        "subagent_tool_count" => {
                            map.next_value_seed(IntegerSeed(&mut out.subagent_tool_count))?;
                        }
                        "subagent_tokens" => {
                            map.next_value_seed(IntegerSeed(&mut out.subagent_tokens))?;
                        }
                        "subagent_duration_ms" => {
                            map.next_value_seed(IntegerSeed(&mut out.subagent_duration_ms))?;
                        }
                        "subagent_cost_usd" => {
                            map.next_value_seed(ElementSeed(&mut out.subagent_cost_usd))?;
                        }
                        "subagent_cost_known" => {
                            map.next_value_seed(ElementSeed(&mut out.subagent_cost_known))?;
                        }
                        "subagent_num_turns" => {
                            map.next_value_seed(IntegerSeed(&mut out.subagent_num_turns))?;
                        }
                        "subagent_cache_read_tokens" => {
                            map.next_value_seed(IntegerSeed(&mut out.subagent_cache_read_tokens))?;
                        }
                        "subagent_cache_create_tokens" => {
                            map.next_value_seed(IntegerSeed(
                                &mut out.subagent_cache_create_tokens,
                            ))?;
                        }
                        "subagent_stop_reason" => {
                            map.next_value_seed(ElementSeed(&mut out.subagent_stop_reason))?;
                        }
                        "subagent_depends_on" => {
                            map.next_value_seed(SliceSeed(&mut subagent_depends_on))?;
                        }
                        "subagent_waiting_on" => {
                            map.next_value_seed(SliceSeed(&mut subagent_waiting_on))?;
                        }
                        "subagent_current_step" => {
                            map.next_value_seed(ElementSeed(&mut out.subagent_current_step))?;
                        }
                        "subagent_last_tool" => {
                            map.next_value_seed(ElementSeed(&mut out.subagent_last_tool))?;
                        }
                        "subagent_files_written" => {
                            map.next_value_seed(IntegerSeed(&mut out.subagent_files_written))?;
                        }
                        "subagent_messages_received" => {
                            map.next_value_seed(IntegerSeed(&mut out.subagent_messages_received))?;
                        }
                        "subagent_last_parent_message" => {
                            map.next_value_seed(ElementSeed(
                                &mut out.subagent_last_parent_message,
                            ))?;
                        }
                        "step" => {
                            map.next_value_seed(ElementSeed(&mut out.step))?;
                        }
                        "model" => {
                            map.next_value_seed(ElementSeed(&mut out.model))?;
                        }
                        "permission_mode" => {
                            map.next_value_seed(ElementSeed(&mut out.permission_mode))?;
                        }
                        "cwd" => {
                            map.next_value_seed(ElementSeed(&mut out.cwd))?;
                        }
                        "max_turns" => {
                            map.next_value_seed(IntegerSeed(&mut out.max_turns))?;
                        }
                        "tools" => {
                            map.next_value_seed(SliceSeed(&mut tools))?;
                        }
                        "mcp_servers" => {
                            map.next_value_seed(SliceSeed(&mut mcp_servers))?;
                        }
                        "cost_usd" => {
                            map.next_value_seed(ElementSeed(&mut out.cost_usd))?;
                        }
                        "cost_known" => {
                            map.next_value_seed(ElementSeed(&mut out.cost_known))?;
                        }
                        "input_tokens" => {
                            map.next_value_seed(IntegerSeed(&mut out.input_tokens))?;
                        }
                        "output_tokens" => {
                            map.next_value_seed(IntegerSeed(&mut out.output_tokens))?;
                        }
                        "cache_read_input_tokens" => {
                            map.next_value_seed(IntegerSeed(&mut out.cache_read_input_tokens))?;
                        }
                        "cache_creation_input_tokens" => {
                            map.next_value_seed(IntegerSeed(&mut out.cache_creation_input_tokens))?;
                        }
                        "input_tokens_include_cache" => {
                            map.next_value_seed(ElementSeed(&mut out.input_tokens_include_cache))?;
                        }
                        "input_tokens_include_cache_known" => {
                            map.next_value_seed(ElementSeed(
                                &mut out.input_tokens_include_cache_known,
                            ))?;
                        }
                        "num_turns" => {
                            map.next_value_seed(IntegerSeed(&mut out.num_turns))?;
                        }
                        "duration_ms" => {
                            map.next_value_seed(IntegerSeed(&mut out.duration_ms))?;
                        }
                        "stop_reason" => {
                            map.next_value_seed(ElementSeed(&mut out.stop_reason))?;
                        }
                        "tokens_before" => {
                            map.next_value_seed(IntegerSeed(&mut out.tokens_before))?;
                        }
                        "tokens_after" => {
                            map.next_value_seed(IntegerSeed(&mut out.tokens_after))?;
                        }
                        "hook_name" => {
                            map.next_value_seed(ElementSeed(&mut out.hook_name))?;
                        }
                        "decision" => {
                            map.next_value_seed(ElementSeed(&mut out.decision))?;
                        }
                        "reason" => {
                            map.next_value_seed(ElementSeed(&mut out.reason))?;
                        }
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                out.tools = tools.into_values().unwrap_or_default();
                out.subagent_depends_on = subagent_depends_on.into_values().unwrap_or_default();
                out.subagent_waiting_on = subagent_waiting_on.into_values().unwrap_or_default();
                out.mcp_servers = mcp_servers.into_values().unwrap_or_default();
                Ok(ParsedEvent(out))
            }
        }
        d.deserialize_any(EventVisitor)
    }
}

struct SliceSeed<'a>(&'a mut crate::userinput::GoSlice<String>);
impl<'de> serde::de::DeserializeSeed<'de> for SliceSeed<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        serde::Deserialize::deserialize_in_place(d, self.0)
    }
}

/// Select an SDK subagent status record and copy its progress fields.
///
/// Dependency lists are independently owned; empty lists project to Go's nil
/// slices. This neither emits an event nor converts native runtime observations.
pub fn subagent_event(event: &ContentEvent) -> Option<crate::dto::SubagentStreamEvent> {
    if event.kind != "subagent_status" || event.task_id.is_empty() {
        return None;
    }
    Some(crate::dto::SubagentStreamEvent {
        task_id: event.task_id.clone(),
        agent_name: if event.agent_name.is_empty() {
            event.subagent_type.clone()
        } else {
            event.agent_name.clone()
        },
        status: event.status.clone(),
        message: event.message.clone(),
        tool_use_id: event.tool_use_id.clone(),
        parent_call_id: event.parent_call_id.clone(),
        depends_on: (!event.subagent_depends_on.is_empty())
            .then(|| event.subagent_depends_on.clone()),
        waiting_on: (!event.subagent_waiting_on.is_empty())
            .then(|| event.subagent_waiting_on.clone()),
        current_step: event.subagent_current_step.clone(),
        last_tool: event.subagent_last_tool.clone(),
        files_written: event.subagent_files_written,
        messages_received: event.subagent_messages_received,
        last_parent_message: event.subagent_last_parent_message.clone(),
        tool_count: event.subagent_tool_count,
        tokens: event.subagent_tokens,
        duration_ms: event.subagent_duration_ms,
        cost_usd: event.subagent_cost_usd,
        cost_known: event.subagent_cost_known,
        num_turns: event.subagent_num_turns,
        stop_reason: event.subagent_stop_reason.clone(),
        result_text: event.subagent_result_text.clone(),
    })
}
