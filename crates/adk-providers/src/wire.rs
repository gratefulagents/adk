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
fn text(content: &[Content]) -> Result<String, Error> {
    let mut out = String::new();
    for part in content {
        match part {
            Content::Text { text } => out.push_str(text),
            _ => return Err(unsupported()),
        }
    }
    Ok(out)
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
    for item in &request.input {
        let entry = match item {
            RunItem::Message { message } => {
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
                    let mut entry = json!({"role":role(message.role),"content":content(&parts, protocol, message.role == Role::Assistant)?});
                    if !reasoning.is_empty() {
                        entry["reasoning_content"] = reasoning.into();
                    }
                    entry
                } else {
                    json!({"role":role(message.role),"content":content(&message.content, protocol, message.role == Role::Assistant)?})
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
            RunItem::ToolResult { call_id, output } => match protocol {
                Protocol::Responses => {
                    json!({"type":"function_call_output","call_id":call_id,"output":text(&output.content)?})
                }
                Protocol::Chat => {
                    json!({"role":"tool","tool_call_id":call_id,"content":text(&output.content)?})
                }
                Protocol::Anthropic => {
                    json!({"role":"user","content":[{"type":"tool_result","tool_use_id":call_id,"content":content(&output.content, protocol, false)?,"is_error":output.is_error}]})
                }
            },
            RunItem::Reasoning { reasoning } => match protocol {
                Protocol::Responses => {
                    if !reasoning.signature.is_empty() || !reasoning.redacted_data.is_empty() {
                        return Err(unsupported());
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
                        return Err(unsupported());
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
                if protocol != Protocol::Responses {
                    return Err(unsupported());
                }
                if compaction.encrypted_content.trim().is_empty() {
                    continue;
                }
                let mut entry = json!({"type":"compaction","encrypted_content":compaction.encrypted_content.trim()});
                if !compaction.id.trim().is_empty() {
                    entry["id"] = compaction.id.clone().into();
                }
                entry
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
            for key in ["content", "tool_calls"] {
                if let Some(parts) = entry[key].as_array() {
                    if !previous[key].is_array() {
                        previous[key] = json!([]);
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
        if key == "thinking_budget" && protocol == Protocol::Anthropic {
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
                | "include"
        ) {
            return Err(crate::invalid("unsupported provider setting"));
        }
        body[key] = value.clone();
    }
    if protocol == Protocol::Anthropic {
        crate::anthropic::shape(&mut body, request, crate::auth::AuthMode::ApiKey);
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
    RunItem::Message {
        message: Message {
            role: Role::Assistant,
            content,
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
fn call(value: &Value, id: &str, name: &str, arguments: &str) -> Result<RunItem, Error> {
    let arguments = match &value[arguments] {
        Value::String(raw) => serde_json::from_str(raw)
            .map_err(|_| Error::new(ErrorCategory::ModelBehavior, "invalid tool arguments JSON"))?,
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
    let mut items = Vec::new();
    let end_turn;
    match protocol {
        Protocol::Responses => {
            if matches!(body["status"].as_str(), Some("failed" | "cancelled"))
                || body.get("error").is_some_and(|v| !v.is_null())
            {
                return Err(Error::new(
                    ErrorCategory::Provider,
                    "provider response failed",
                ));
            }
            let output = body["output"].as_array().ok_or_else(|| {
                Error::new(ErrorCategory::Provider, "provider response missing output")
            })?;
            for item in output {
                match item["type"].as_str() {
                    Some("message") => {
                        let mut parts = Vec::new();
                        for part in item["content"].as_array().ok_or_else(|| {
                            Error::new(ErrorCategory::Provider, "invalid response message")
                        })? {
                            match part["type"].as_str() {
                                Some("output_text") => parts.push(Content::Text {
                                    text: string(part, "text")?,
                                }),
                                Some("refusal") => parts.push(Content::Text {
                                    text: string(part, "refusal")?,
                                }),
                                _ => return Err(unsupported()),
                            }
                        }
                        items.push(message(parts));
                    }
                    Some("function_call") => {
                        items.push(call(item, "call_id", "name", "arguments")?)
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
                    Some("compaction") => items.push(RunItem::Compaction {
                        compaction: adk_core::Compaction {
                            id: item["id"].as_str().unwrap_or_default().to_owned(),
                            encrypted_content: item["encrypted_content"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned(),
                            content: item["content"].as_str().unwrap_or_default().to_owned(),
                            created_by: item["created_by"].as_str().unwrap_or_default().to_owned(),
                        },
                    }),
                    _ => return Err(unsupported()),
                }
            }
            end_turn = Some(!items.iter().any(|v| matches!(v, RunItem::ToolCall { .. })));
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
            let reasoning = ["reasoning", "reasoning_content", "reasoning_text"]
                .iter()
                .filter_map(|key| msg[key].as_str())
                .find(|value| !value.trim().is_empty())
                .unwrap_or_default();
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
            if !parts.is_empty() {
                items.push(message(parts));
            }
            if let Some(calls) = msg["tool_calls"].as_array() {
                for tool in calls {
                    let mut function = tool["function"].clone();
                    function["id"] = tool["id"].clone();
                    items.push(call(&function, "id", "name", "arguments")?);
                }
            }
            end_turn = Some(choice["finish_reason"] != "tool_calls");
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
                    Some("tool_use") => items.push(call(part, "id", "name", "input")?),
                    _ => return Err(unsupported()),
                }
            }
            end_turn = Some(body["stop_reason"] != "tool_use");
        }
    }
    Ok(ModelResponse {
        items,
        usage: usage(&body["usage"], protocol),
        end_turn,
        response_id: body["id"].as_str().map(str::to_owned),
        metadata: body["metadata"].as_object().cloned().unwrap_or_default(),
    })
}
