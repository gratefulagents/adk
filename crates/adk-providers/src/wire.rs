//! Explicit wire conversion, independent of HTTP and credential storage.
use adk_core::{
    Content, Error, ErrorCategory, Message, ModelRequest, ModelResponse, Role, RunItem, ToolCall,
    Usage,
};
use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use serde_json::{Value, json};
const DETAILS_PREFIX: &str = "openrouter-reasoning-details:";

pub fn encode_reasoning_details(details: &Value) -> String {
    format!(
        "{DETAILS_PREFIX}{}",
        STANDARD_NO_PAD.encode(details.to_string())
    )
}
pub(crate) fn reasoning_details_text(details: &Value) -> String {
    details
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|detail| {
            detail["text"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| detail["summary"].as_str())
        })
        .collect()
}
pub fn decode_reasoning_details(signature: &str) -> Option<Value> {
    let bytes = STANDARD_NO_PAD
        .decode(signature.strip_prefix(DETAILS_PREFIX)?)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Responses,
    Chat,
    Anthropic,
}
impl Protocol {
    pub fn path(self) -> &'static str {
        match self {
            Self::Responses => "/responses",
            Self::Chat => "/chat/completions",
            Self::Anthropic => "/v1/messages",
        }
    }
}
fn unsupported() -> Error {
    Error::new(
        ErrorCategory::Unsupported,
        "content is not representable by this provider protocol",
    )
}
fn role(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::Developer => "developer",
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}
fn content(parts: &[Content], protocol: Protocol, assistant: bool) -> Result<Vec<Value>, Error> {
    parts.iter().map(|part| Ok(match (protocol, part) {
        (Protocol::Responses, Content::Text { text }) => json!({"type":if assistant {"output_text"} else {"input_text"},"text":text}),
        (_, Content::Text { text }) => json!({"type":"text","text":text}),
        (Protocol::Chat, Content::Image { uri, .. }) => json!({"type":"image_url","image_url":{"url":uri}}),
        (Protocol::Responses, Content::Image { uri, .. }) => json!({"type":"input_image","image_url":uri}),
        (protocol, Content::Attachment { media_type, data, detail }) => {
            if media_type == "application/pdf" {
                if protocol != Protocol::Anthropic { return Err(unsupported()); }
                json!({"type":"document","source":{"type":"base64","media_type":media_type,"data":data}})
            } else if media_type.starts_with("image/") {
                let uri = format!("data:{media_type};base64,{data}");
                match protocol {
                    Protocol::Anthropic => json!({"type":"image","source":{"type":"base64","media_type":media_type,"data":data}}),
                    Protocol::Responses => json!({"type":"input_image","image_url":uri,"detail":if detail.is_empty() { "auto" } else { detail }}),
                    Protocol::Chat => {
                        let mut image = json!({"type":"image_url","image_url":{"url":uri}});
                        if !detail.is_empty() { image["image_url"]["detail"] = detail.clone().into(); }
                        image
                    }
                }
            } else { return Err(unsupported()); }
        },
        (Protocol::Anthropic, Content::Image { uri, media_type }) => {
            if let Some((prefix, data)) = uri.strip_prefix("data:").and_then(|v| v.split_once(',')) {
                if !prefix.ends_with(";base64") { return Err(unsupported()); }
                json!({"type":"image","source":{"type":"base64","media_type":media_type,"data":data}})
            } else { json!({"type":"image","source":{"type":"url","url":uri}}) }
        }
        (Protocol::Anthropic, Content::Reasoning { text, signature: Some(signature) }) => json!({"type":"thinking","thinking":text,"signature":signature}),
        _ => return Err(unsupported()),
    })).collect()
}

fn message_content(parts: &[Content], protocol: Protocol, assistant: bool) -> Result<Value, Error> {
    if protocol != Protocol::Anthropic
        && parts
            .iter()
            .all(|part| matches!(part, Content::Text { .. }))
    {
        let text = parts
            .iter()
            .filter_map(|part| match part {
                Content::Text { text } if !text.trim().is_empty() => {
                    Some(if protocol == Protocol::Responses {
                        text.trim()
                    } else {
                        text.as_str()
                    })
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(if text.is_empty() && assistant {
            Value::Null
        } else {
            text.into()
        });
    }
    Ok(content(parts, protocol, assistant)?.into())
}
pub fn request(request: &ModelRequest, protocol: Protocol, stream: bool) -> Result<Value, Error> {
    if request.model.trim().is_empty() {
        return Err(crate::invalid("model name is empty"));
    }
    let mut entries = Vec::new();
    let mut system = Vec::new();
    if !request.instructions.is_empty() {
        if protocol == Protocol::Anthropic {
            system.push(json!({"type":"text","text":request.instructions}));
        } else if protocol == Protocol::Chat {
            entries.push(json!({"role":"system","content":request.instructions}));
        }
    }
    let mut tool_images = Vec::new();
    for (input_index, item) in request.input.iter().enumerate() {
        if !matches!(item, RunItem::ToolResult { .. }) && !tool_images.is_empty() {
            entries.push(json!({"role":"user","content":content(&tool_images, protocol, false)?}));
            tool_images.clear();
        }
        let entry = match item {
            RunItem::Message { message } | RunItem::PhasedMessage { message, .. } => {
                if protocol == Protocol::Anthropic
                    && matches!(message.role, Role::System | Role::Developer)
                {
                    system.extend(content(&message.content, protocol, false)?);
                    continue;
                }
                if protocol == Protocol::Chat {
                    let mut parts = Vec::new();
                    let mut reasoning = String::new();
                    for part in &message.content {
                        if let Content::Reasoning { text, .. } = part {
                            reasoning.push_str(text);
                        } else {
                            parts.push(part.clone());
                        }
                    }
                    let mut entry = json!({"role":role(message.role),"content":message_content(&parts, protocol, message.role == Role::Assistant)?});
                    if !reasoning.is_empty() {
                        entry["reasoning_content"] = reasoning.into();
                    }
                    entry
                } else {
                    let mut entry = json!({"role":role(message.role),"content":message_content(&message.content, protocol, message.role == Role::Assistant)?});
                    if protocol == Protocol::Responses && message.role == Role::Assistant {
                        let phase = match item {
                            RunItem::PhasedMessage { phase, .. } => phase.as_str(),
                            _ if matches!(
                                request.input.get(input_index + 1),
                                Some(RunItem::ToolCall { .. })
                            ) =>
                            {
                                "commentary"
                            }
                            _ => "final_answer",
                        };
                        entry["phase"] = phase.into();
                    }
                    entry
                }
            }
            RunItem::ToolCall { call } => match protocol {
                Protocol::Responses => {
                    json!({"type":"function_call","call_id":call.id,"name":call.name,"arguments":call.arguments.to_string()})
                }
                Protocol::Chat => {
                    json!({"role":"assistant","content":null,"tool_calls":[{"id":call.id,"type":"function","function":{"name":call.name,"arguments":call.arguments.to_string()}}]})
                }
                Protocol::Anthropic => {
                    json!({"role":"assistant","content":[{"type":"tool_use","id":call.id,"name":call.name,"input":call.arguments}]})
                }
            },
            RunItem::ToolResult { call_id, output } => {
                if protocol == Protocol::Anthropic {
                    json!({"role":"user","content":[{"type":"tool_result","tool_use_id":call_id,"content":content(&output.content, protocol, false)?,"is_error":output.is_error}]})
                } else {
                    let mut texts = Vec::new();
                    for part in &output.content {
                        match part {
                            Content::Text { text } => texts.push(text.as_str()),
                            Content::Image { .. } | Content::Attachment { .. } => {
                                tool_images.push(part.clone())
                            }
                            _ => return Err(unsupported()),
                        }
                    }
                    let text = texts.join("\n");
                    let text = if text.is_empty() {
                        "(no output)"
                    } else {
                        &text
                    };
                    if protocol == Protocol::Responses {
                        json!({"type":"function_call_output","call_id":call_id,"output":text})
                    } else {
                        json!({"role":"tool","tool_call_id":call_id,"content":text})
                    }
                }
            }
            RunItem::Reasoning { reasoning } => match protocol {
                Protocol::Responses => {
                    if !reasoning.signature.is_empty() || !reasoning.redacted_data.is_empty() {
                        continue;
                    }
                    if reasoning.encrypted_content.trim().is_empty() {
                        continue;
                    }
                    let id = if reasoning.id.trim().is_empty() {
                        format!("reasoning_{}", entries.len())
                    } else {
                        reasoning.id.clone()
                    };
                    // Codex requires summary:[] and rejects status on replay.
                    json!({"type":"reasoning","id":id,"summary":[],"encrypted_content":reasoning.encrypted_content})
                }
                Protocol::Anthropic => {
                    if !reasoning.encrypted_content.is_empty()
                        || reasoning.signature.starts_with(DETAILS_PREFIX)
                    {
                        continue;
                    }
                    if !reasoning.redacted_data.is_empty() {
                        json!({"role":"assistant","content":[{"type":"redacted_thinking","data":reasoning.redacted_data}]})
                    } else if !reasoning.signature.is_empty() {
                        json!({"role":"assistant","content":[{"type":"thinking","thinking":reasoning.text,"signature":reasoning.signature}]})
                    } else {
                        continue;
                    }
                }
                Protocol::Chat => {
                    if !reasoning.encrypted_content.is_empty()
                        || !reasoning.redacted_data.is_empty()
                    {
                        return Err(unsupported());
                    }
                    if reasoning.text.is_empty() && reasoning.signature.is_empty() {
                        continue;
                    }
                    let mut entry = json!({"role":"assistant","content":null});
                    if let Some(details) = decode_reasoning_details(&reasoning.signature) {
                        entry["reasoning_details"] = details;
                    } else if !reasoning.signature.is_empty() {
                        entry["reasoning_text"] = reasoning.text.clone().into();
                        entry["reasoning_opaque"] = reasoning.signature.clone().into();
                    } else {
                        entry["reasoning"] = reasoning.text.clone().into();
                    }
                    entry
                }
            },
            RunItem::Compaction { compaction } => {
                let native = if protocol == Protocol::Anthropic {
                    "anthropic"
                } else {
                    "openai"
                };
                let origin = compaction.created_by.trim().to_lowercase();
                if protocol == Protocol::Chat
                    || (!origin.is_empty() && origin != native)
                    || compaction.encrypted_content.trim().is_empty()
                {
                    if compaction.content.trim().is_empty() {
                        continue;
                    }
                    let summary = format!(
                        "[CONTEXT SUMMARY carried over from an earlier context compaction by a different model provider]\n{}",
                        compaction.content.trim()
                    );
                    if protocol == Protocol::Anthropic {
                        json!({"role":"assistant","content":[{"type":"text","text":summary}]})
                    } else {
                        json!({"role":"assistant","content":summary})
                    }
                } else if protocol == Protocol::Anthropic {
                    let summary = if compaction.content.is_empty() {
                        Value::Null
                    } else {
                        compaction.content.clone().into()
                    };
                    json!({"role":"assistant","content":[{"type":"compaction","content":summary,"encrypted_content":compaction.encrypted_content}]})
                } else {
                    let mut entry = json!({"type":"compaction","encrypted_content":compaction.encrypted_content.trim()});
                    if !compaction.id.trim().is_empty() {
                        entry["id"] = compaction.id.clone().into();
                    }
                    entry
                }
            }
            RunItem::Handoff { .. } => return Err(unsupported()),
        };
        if protocol == Protocol::Anthropic
            && entries
                .last()
                .is_some_and(|previous: &Value| previous["role"] == entry["role"])
        {
            entries.last_mut().unwrap()["content"]
                .as_array_mut()
                .unwrap()
                .extend(entry["content"].as_array().unwrap().clone());
        } else if protocol == Protocol::Chat
            && entry["role"] == "assistant"
            && entries
                .last()
                .is_some_and(|previous| previous["role"] == "assistant")
        {
            let previous = entries.last_mut().unwrap();
            if let Some(text) = entry["content"].as_str() {
                if let Some(parts) = previous["content"].as_array_mut() {
                    parts.push(json!({"type":"text","text":text}));
                } else if let Some(before) = previous["content"].as_str().filter(|v| !v.is_empty())
                {
                    previous["content"] = format!("{before}\n{text}").into();
                } else {
                    previous["content"] = text.into();
                }
            }
            for key in ["content", "tool_calls"] {
                if let Some(parts) = entry[key].as_array() {
                    if !previous[key].is_array() {
                        previous[key] = if key == "content" {
                            previous[key]
                                .as_str()
                                .map(|text| json!([{"type":"text","text":text}]))
                                .unwrap_or_else(|| json!([]))
                        } else {
                            json!([])
                        };
                    }
                    previous[key]
                        .as_array_mut()
                        .unwrap()
                        .extend(parts.iter().cloned());
                }
            }
            if let Some(details) = entry.get("reasoning_details") {
                previous["reasoning_details"] = details.clone();
            }
            for key in [
                "reasoning_content",
                "reasoning",
                "reasoning_text",
                "reasoning_opaque",
            ] {
                if let Some(value) = entry[key].as_str() {
                    previous[key] =
                        format!("{}{}", previous[key].as_str().unwrap_or_default(), value).into();
                }
            }
        } else {
            entries.push(entry);
        }
    }
    if !tool_images.is_empty() {
        entries.push(json!({"role":"user","content":content(&tool_images, protocol, false)?}));
    }
    let tools: Vec<Value> = request.tools.iter().map(|tool| match protocol {
        Protocol::Responses => json!({"type":"function","name":tool.name,"description":tool.description,"parameters":tool.input_schema}),
        Protocol::Chat => json!({"type":"function","function":{"name":tool.name,"description":tool.description,"parameters":tool.input_schema}}),
        Protocol::Anthropic => json!({"name":tool.name,"description":tool.description,"input_schema":tool.input_schema}),
    }).collect();
    let mut body = json!({"model":request.model,"stream":stream});
    match protocol {
        Protocol::Responses => {
            body["input"] = entries.into();
            body["instructions"] = request.instructions.clone().into();
            body["store"] = false.into();
        }
        Protocol::Chat => {
            body["messages"] = entries.into();
            if stream {
                body["stream_options"] = json!({"include_usage":true});
            }
        }
        Protocol::Anthropic => {
            body["messages"] = entries.into();
            body["system"] = system.into();
            body["max_tokens"] = json!(16384);
        }
    }
    if !tools.is_empty() {
        body["tools"] = tools.into();
    }
    if let Some(schema) = &request.output_schema {
        let format = json!({"type":"json_schema","name":request.output_schema_name,"strict":request.output_schema_strict,"schema":schema});
        match protocol {
            Protocol::Responses => body["text"] = json!({"format":format}),
            Protocol::Chat => {
                body["response_format"] = json!({"type":"json_schema","json_schema":{"name":request.output_schema_name,"strict":request.output_schema_strict,"schema":schema}})
            }
            Protocol::Anthropic => {
                body["output_format"] = json!({"type":"json_schema","schema":schema})
            }
        }
    }
    // Structural fields are not replaceable through an untyped settings escape hatch.
    for (key, value) in &request.settings {
        if matches!(
            key.as_str(),
            "model_fallbacks" | "text_verbosity" | "compaction_threshold"
        ) {
            if protocol == Protocol::Anthropic {
                return Err(crate::invalid(
                    "setting requires OpenAI-compatible protocol",
                ));
            }
            body[key] = value.clone();
            continue;
        }
        if key == "thinking_budget" {
            if !value.is_u64() {
                return Err(crate::invalid(
                    "thinking budget must be a nonnegative integer",
                ));
            }
            body[key] = value.clone();
            continue;
        }
        if !matches!(
            key.as_str(),
            "temperature"
                | "top_p"
                | "max_tokens"
                | "max_output_tokens"
                | "max_completion_tokens"
                | "reasoning"
                | "reasoning_effort"
                | "thinking"
                | "metadata"
                | "prompt_cache_key"
                | "prompt_cache_retention"
                | "parallel_tool_calls"
                | "tool_choice"
                | "stop"
                | "service_tier"
                | "truncation"
                | "include"
        ) {
            return Err(crate::invalid("unsupported provider setting"));
        }
        body[key] = value.clone();
    }
    if protocol == Protocol::Anthropic {
        crate::anthropic::shape(&mut body, request, crate::auth::AuthMode::ApiKey);
    } else {
        crate::openai::shape(&mut body, request, protocol)?;
    }
    Ok(body)
}

pub fn usage(value: &Value, protocol: Protocol) -> Usage {
    let number = |key: &str| value.pointer(key).and_then(Value::as_u64).unwrap_or(0);
    let (input, output, read, creation) = match protocol {
        Protocol::Responses => (
            number("/input_tokens"),
            number("/output_tokens"),
            number("/input_tokens_details/cached_tokens"),
            number("/input_tokens_details/cache_write_tokens"),
        ),
        Protocol::Chat => (
            number("/prompt_tokens"),
            number("/completion_tokens"),
            number("/prompt_tokens_details/cached_tokens"),
            number("/prompt_tokens_details/cache_write_tokens"),
        ),
        Protocol::Anthropic => (
            number("/input_tokens"),
            number("/output_tokens"),
            number("/cache_read_input_tokens"),
            number("/cache_creation_input_tokens"),
        ),
    };
    Usage {
        requests: 1,
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: read,
        cache_creation_tokens: creation,
        context_tokens: Some(if protocol == Protocol::Anthropic {
            input.saturating_add(read).saturating_add(creation)
        } else {
            input
        }),
    }
}
fn message(content: Vec<Content>) -> RunItem {
    phased_message(content, Role::Assistant, None)
}
fn phased_message(content: Vec<Content>, role: Role, phase: Option<&str>) -> RunItem {
    let message = Message { role, content };
    match phase.filter(|phase| !phase.is_empty()) {
        Some(phase) => RunItem::PhasedMessage {
            message,
            phase: phase.to_owned(),
        },
        None => RunItem::Message { message },
    }
}
fn compaction(item: &Value, origin: &str) -> RunItem {
    RunItem::Compaction {
        compaction: adk_core::Compaction {
            id: item["id"].as_str().unwrap_or_default().to_owned(),
            encrypted_content: item["encrypted_content"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            content: item["content"].as_str().unwrap_or_default().to_owned(),
            created_by: item["created_by"]
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(origin)
                .to_owned(),
        },
    }
}
fn string(value: &Value, field: &str) -> Result<String, Error> {
    value[field].as_str().map(str::to_owned).ok_or_else(|| {
        Error::new(
            ErrorCategory::Provider,
            "provider response missing required string",
        )
    })
}
fn call(
    value: &Value,
    id: &str,
    name: &str,
    arguments: &str,
    normalize: bool,
) -> Result<RunItem, Error> {
    let arguments = match &value[arguments] {
        Value::String(raw) if raw.trim().is_empty() => json!({}),
        Value::String(raw) => match serde_json::from_str(raw) {
            Ok(value) => value,
            Err(_) if normalize => json!({}),
            Err(_) => {
                return Err(Error::new(
                    ErrorCategory::ModelBehavior,
                    "invalid tool arguments JSON",
                ));
            }
        },
        value if value.is_object() => value.clone(),
        _ => {
            return Err(Error::new(
                ErrorCategory::ModelBehavior,
                "invalid tool arguments",
            ));
        }
    };
    Ok(RunItem::ToolCall {
        call: ToolCall {
            id: string(value, id)?,
            name: string(value, name)?,
            arguments,
        },
    })
}
pub fn response(body: &Value, protocol: Protocol) -> Result<ModelResponse, Error> {
    if let Some(error) = crate::error::provider_error(body) {
        return Err(error);
    }
    let mut items = Vec::new();
    let end_turn;
    match protocol {
        Protocol::Responses => {
            if matches!(
                body["status"].as_str(),
                Some("failed" | "cancelled" | "canceled")
            ) {
                return Err(crate::error::response_error(body, false).expect("failed response"));
            }
            let output = body["output"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            for (index, item) in output.iter().enumerate() {
                match item["type"].as_str() {
                    Some("message") => {
                        let mut parts = Vec::new();
                        for part in item["content"].as_array().ok_or_else(|| {
                            Error::new(ErrorCategory::Provider, "invalid response message")
                        })? {
                            match part["type"].as_str() {
                                Some("output_text" | "input_text" | "text") => {
                                    parts.push(Content::Text {
                                        text: string(part, "text")?,
                                    })
                                }
                                Some("refusal") => parts.push(Content::Text {
                                    text: format!(
                                        "The model refused to respond: {}",
                                        string(part, "refusal")?
                                    ),
                                }),
                                _ => return Err(unsupported()),
                            }
                        }
                        let role = match item["role"].as_str() {
                            Some("user") => Role::User,
                            Some("system") => Role::System,
                            Some("developer") => Role::Developer,
                            _ => Role::Assistant,
                        };
                        parts.retain(|part| matches!(part, Content::Text { text } if !text.trim().is_empty()));
                        if !parts.is_empty() {
                            items.push(phased_message(parts, role, item["phase"].as_str()));
                        }
                    }
                    Some("function_call_output") => {
                        let output = item["output"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| item["output"].to_string());
                        items.push(RunItem::ToolResult {
                            call_id: string(item, "call_id")?,
                            output: adk_core::ToolOutput {
                                content: vec![Content::Text { text: output }],
                                is_error: false,
                                should_pause: false,
                            },
                        });
                    }
                    Some(
                        "function_call"
                        | "web_search_call"
                        | "file_search_call"
                        | "code_interpreter_call"
                        | "mcp_call"
                        | "computer_call"
                        | "image_generation_call"
                        | "tool_search_call"
                        | "local_shell_call"
                        | "shell_call"
                        | "apply_patch_call"
                        | "custom_tool_call",
                    ) => {
                        let mut tool = item.clone();
                        let kind = item["type"].as_str().unwrap();
                        if tool["call_id"]
                            .as_str()
                            .is_none_or(|id| id.trim().is_empty())
                        {
                            tool["call_id"] = if kind == "function_call" {
                                format!("call_{index}")
                            } else {
                                format!("call_{kind}_{index}")
                            }
                            .into();
                        }
                        if tool["name"].as_str().is_none_or(str::is_empty)
                            && kind != "function_call"
                        {
                            tool["name"] = kind.into();
                        }
                        if tool.get("arguments").is_none() {
                            tool["arguments"] = json!({});
                        }
                        items.push(call(&tool, "call_id", "name", "arguments", true)?);
                    }
                    Some("reasoning") => {
                        let text = item["summary"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|part| part["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        let encrypted_content = item["encrypted_content"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned();
                        if !text.is_empty() || !encrypted_content.is_empty() {
                            items.push(RunItem::Reasoning {
                                reasoning: adk_core::Reasoning {
                                    id: item["id"].as_str().unwrap_or_default().to_owned(),
                                    text,
                                    encrypted_content,
                                    ..Default::default()
                                },
                            });
                        }
                    }
                    Some("compaction") => items.push(compaction(item, "openai")),
                    _ => return Err(unsupported()),
                }
            }
            if items.is_empty()
                && let Some(text) = body["output_text"].as_str().filter(|text| !text.is_empty())
            {
                items.push(message(vec![Content::Text {
                    text: text.to_owned(),
                }]));
            }
            if let Some(error) = crate::error::response_error(body, !items.is_empty()) {
                return Err(error);
            }
            end_turn = body["end_turn"].as_bool();
        }
        Protocol::Chat => {
            let choice = &body["choices"][0];
            let msg = &choice["message"];
            if !msg.is_object() {
                return Err(Error::new(
                    ErrorCategory::Provider,
                    "chat response missing message",
                ));
            }
            let mut parts = Vec::new();
            let details_text = reasoning_details_text(&msg["reasoning_details"]);
            let reasoning = ["reasoning", "reasoning_content", "reasoning_text"]
                .iter()
                .filter_map(|key| msg[key].as_str())
                .find(|value| !value.trim().is_empty())
                .unwrap_or(&details_text);
            let signature = msg
                .get("reasoning_details")
                .filter(|value| !value.is_null())
                .map(encode_reasoning_details)
                .unwrap_or_else(|| {
                    msg["reasoning_opaque"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned()
                });
            if !reasoning.is_empty() || !signature.is_empty() {
                items.push(RunItem::Reasoning {
                    reasoning: adk_core::Reasoning {
                        text: reasoning.to_owned(),
                        signature,
                        ..Default::default()
                    },
                });
            }
            if let Some(text) = msg["content"]
                .as_str()
                .filter(|text| !text.trim().is_empty())
            {
                parts.push(Content::Text {
                    text: text.to_owned(),
                });
            }
            if let Some(content) = msg["content"].as_array() {
                for part in content {
                    if part["type"] == "text"
                        && let Some(text) = part["text"].as_str().filter(|text| !text.is_empty())
                    {
                        parts.push(Content::Text {
                            text: text.to_owned(),
                        });
                    }
                }
            }
            if let Some(refusal) = msg["refusal"].as_str().filter(|text| !text.is_empty()) {
                parts.push(Content::Text {
                    text: if parts.is_empty() {
                        format!("The model refused to respond: {refusal}")
                    } else {
                        refusal.to_owned()
                    },
                });
            }
            if !parts.is_empty() {
                items.push(message(parts));
            }
            if let Some(calls) = msg["tool_calls"].as_array() {
                for (index, tool) in calls.iter().enumerate() {
                    let mut function = tool["function"].clone();
                    function["id"] = tool["id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .map_or_else(|| format!("call_{index}"), str::to_owned)
                        .into();
                    if function.get("arguments").is_none() {
                        function["arguments"] = json!({});
                    }
                    items.push(call(&function, "id", "name", "arguments", true)?);
                }
            }
            end_turn = body["end_turn"].as_bool().or_else(|| {
                Some(
                    !items
                        .iter()
                        .any(|item| matches!(item, RunItem::ToolCall { .. })),
                )
            });
        }
        Protocol::Anthropic => {
            for part in body["content"].as_array().ok_or_else(|| {
                Error::new(
                    ErrorCategory::Provider,
                    "Anthropic response missing content",
                )
            })? {
                match part["type"].as_str() {
                    Some("text") => items.push(message(vec![Content::Text {
                        text: string(part, "text")?,
                    }])),
                    Some("thinking") => items.push(RunItem::Reasoning {
                        reasoning: adk_core::Reasoning {
                            text: string(part, "thinking")?,
                            signature: part["signature"].as_str().unwrap_or_default().to_owned(),
                            ..Default::default()
                        },
                    }),
                    Some("redacted_thinking") => items.push(RunItem::Reasoning {
                        reasoning: adk_core::Reasoning {
                            redacted_data: string(part, "data")?,
                            ..Default::default()
                        },
                    }),
                    Some("tool_use") => items.push(call(part, "id", "name", "input", false)?),
                    Some("compaction") => items.push(compaction(part, "anthropic")),
                    _ => return Err(unsupported()),
                }
            }
            end_turn = body["end_turn"].as_bool().or_else(|| {
                Some(!matches!(
                    body["stop_reason"].as_str(),
                    Some("tool_use" | "pause_turn")
                ))
            });
        }
    }
    Ok(ModelResponse {
        raw: Some(body.clone()),
        items,
        usage: usage(&body["usage"], protocol),
        end_turn,
        response_id: body["id"].as_str().map(str::to_owned),
        metadata: body["metadata"].as_object().cloned().unwrap_or_default(),
    })
}
