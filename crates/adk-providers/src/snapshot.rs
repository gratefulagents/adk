//! Ordered provider-neutral diagnostic documents used by compatibility traces.
//! Native provider payloads remain separately available on `ModelResponse::raw`.
use crate::wire::Protocol;
use adk_core::{Error, ErrorCategory, JsonDocument};
use serde::{Deserialize, Serialize};
use serde_json::{Value, value::RawValue};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Response {
    #[serde(deserialize_with = "null_default")]
    id: String,
    #[serde(rename = "type")]
    #[serde(deserialize_with = "null_default")]
    kind: String,
    #[serde(deserialize_with = "null_default")]
    role: String,
    content: Option<Vec<Block>>,
    #[serde(deserialize_with = "null_default")]
    model: String,
    #[serde(deserialize_with = "null_default")]
    stop_reason: String,
    #[serde(deserialize_with = "null_default")]
    usage: Usage,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_turn: Option<bool>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Block {
    #[serde(rename = "type")]
    #[serde(deserialize_with = "null_default")]
    kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    phase: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    thinking: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    signature: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    data: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    encrypted_content: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    created_by: String,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_input"
    )]
    input: Option<Box<RawValue>>,
    #[serde(skip_serializing_if = "empty_images")]
    result_images: Option<Vec<Image>>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    tool_use_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    content: String,
    #[serde(skip_serializing_if = "is_false")]
    #[serde(deserialize_with = "null_default")]
    is_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<Image>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Image {
    #[serde(rename = "type")]
    #[serde(deserialize_with = "null_default")]
    kind: String,
    #[serde(deserialize_with = "null_default")]
    media_type: String,
    #[serde(deserialize_with = "null_default")]
    data: String,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct CacheControl {
    #[serde(rename = "type")]
    #[serde(deserialize_with = "null_default")]
    kind: String,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Usage {
    #[serde(deserialize_with = "null_default")]
    input_tokens: i64,
    #[serde(deserialize_with = "null_default")]
    output_tokens: i64,
    #[serde(skip_serializing_if = "is_zero")]
    #[serde(deserialize_with = "null_default")]
    cache_read_input_tokens: i64,
    #[serde(skip_serializing_if = "is_zero")]
    #[serde(deserialize_with = "null_default")]
    cache_creation_input_tokens: i64,
}
fn null_default<'de, D: serde::Deserializer<'de>, T: Deserialize<'de> + Default>(
    deserializer: D,
) -> Result<T, D::Error> {
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}
fn present_input<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Box<RawValue>>, D::Error> {
    Box::<RawValue>::deserialize(deserializer).map(Some)
}
fn empty_images(images: &Option<Vec<Image>>) -> bool {
    images.as_ref().is_none_or(Vec::is_empty)
}
fn is_zero(value: &i64) -> bool {
    *value == 0
}
fn is_false(value: &bool) -> bool {
    !value
}
fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}
fn invalid() -> Error {
    Error::new(
        ErrorCategory::Provider,
        "invalid normalized provider snapshot",
    )
}
fn arguments(value: &Value) -> Box<RawValue> {
    if value.is_object() {
        // Native adapters also accept already-decoded argument objects.
        return RawValue::from_string(value.to_string()).expect("serialized JSON object");
    }
    let input = value.as_str().map(str::trim).unwrap_or_default();
    // Match the native call parser before retaining lexical JSON. RawValue alone
    // accepts escapes (such as an unpaired surrogate) that Value rejects; those
    // model-generated arguments already normalize to {} in the native response.
    let input = if serde_json::from_str::<Value>(input).is_ok() {
        input
    } else {
        "{}"
    };
    RawValue::from_string(input.to_owned())
        .unwrap_or_else(|_| RawValue::from_string("{}".into()).unwrap())
}
fn block_text(text: String, phase: String) -> Block {
    Block {
        kind: "text".into(),
        text,
        phase,
        ..Default::default()
    }
}

/// `source` retains original Anthropic RawMessage fields on the HTTP path.
/// Value-only callers have already chosen a canonical map representation.
pub(crate) fn document(
    body: &Value,
    protocol: Protocol,
    source: Option<&str>,
) -> Result<JsonDocument, Error> {
    let response = if protocol == Protocol::Anthropic {
        let encoded;
        let source = match source {
            Some(source) => source,
            None => {
                encoded = body.to_string();
                &encoded
            }
        };
        serde_json::from_str::<Response>(source).map_err(|_| invalid())?
    } else {
        let mut response = Response {
            id: text(body, "id"),
            kind: "message".into(),
            role: "assistant".into(),
            model: text(body, "model"),
            ..Default::default()
        };
        let mut blocks = Vec::new();
        match protocol {
            Protocol::Responses => {
                let usage = &body["usage"];
                response.usage = Usage {
                    input_tokens: usage["input_tokens"].as_i64().unwrap_or_default(),
                    output_tokens: usage["output_tokens"].as_i64().unwrap_or_default(),
                    cache_read_input_tokens: usage["input_tokens_details"]["cached_tokens"]
                        .as_i64()
                        .unwrap_or_default(),
                    cache_creation_input_tokens:
                        usage["input_tokens_details"]["cache_write_tokens"]
                            .as_i64()
                            .unwrap_or_default(),
                };
                response.end_turn = body["end_turn"].as_bool();
                for (index, item) in body["output"].as_array().into_iter().flatten().enumerate() {
                    let kind = item["type"].as_str().unwrap_or_default();
                    match kind {
                        "message" => {
                            for part in item["content"].as_array().into_iter().flatten() {
                                let kind = part["type"].as_str().unwrap_or_default();
                                let value = text(part, "text");
                                if matches!(kind, "output_text" | "input_text" | "")
                                    && !value.trim().is_empty()
                                {
                                    blocks.push(block_text(value, text(item, "phase")));
                                }
                                let refusal = text(part, "refusal");
                                if kind == "refusal" && !refusal.trim().is_empty() {
                                    blocks.push(block_text(
                                        format!("The model refused to respond: {refusal}"),
                                        text(item, "phase"),
                                    ));
                                }
                            }
                        }
                        "reasoning" => {
                            let mut thinking = String::new();
                            for part in item["content"].as_array().into_iter().flatten() {
                                if matches!(
                                    part["type"].as_str().unwrap_or_default(),
                                    "reasoning_text" | "output_text" | ""
                                ) {
                                    let value = text(part, "text");
                                    if !value.trim().is_empty() {
                                        thinking.push_str(&value);
                                    }
                                }
                            }
                            if thinking.is_empty() {
                                for part in item["summary"].as_array().into_iter().flatten() {
                                    let value = text(part, "text");
                                    if !value.trim().is_empty() {
                                        thinking.push_str(&value);
                                    }
                                }
                            }
                            let encrypted_content = text(item, "encrypted_content");
                            if !thinking.is_empty() || !encrypted_content.is_empty() {
                                blocks.push(Block {
                                    kind: "thinking".into(),
                                    id: text(item, "id"),
                                    thinking,
                                    encrypted_content,
                                    ..Default::default()
                                });
                            }
                        }
                        "compaction" => blocks.push(Block {
                            kind: kind.into(),
                            id: text(item, "id"),
                            encrypted_content: text(item, "encrypted_content"),
                            created_by: text(item, "created_by"),
                            ..Default::default()
                        }),
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
                        | "custom_tool_call" => {
                            let mut id = text(item, "call_id").trim().to_owned();
                            if id.is_empty() {
                                id = if kind == "function_call" {
                                    format!("call_{index}")
                                } else {
                                    format!("call_{kind}_{index}")
                                };
                            }
                            let mut name = text(item, "name");
                            if name.is_empty() && kind != "function_call" {
                                name = kind.into();
                            }
                            blocks.push(Block {
                                kind: "tool_use".into(),
                                id,
                                name,
                                input: Some(arguments(&item["arguments"])),
                                ..Default::default()
                            });
                        }
                        _ => {}
                    }
                }
                response.stop_reason = if body["incomplete_details"]["reason"]
                    .as_str()
                    .is_some_and(|s| s.eq_ignore_ascii_case("max_output_tokens"))
                {
                    "max_tokens"
                } else if blocks.iter().any(|b| b.kind == "tool_use") {
                    "tool_use"
                } else {
                    "end_turn"
                }
                .into();
            }
            Protocol::Chat => {
                let usage = &body["usage"];
                response.usage = Usage {
                    input_tokens: usage["prompt_tokens"].as_i64().unwrap_or_default(),
                    output_tokens: usage["completion_tokens"].as_i64().unwrap_or_default(),
                    cache_read_input_tokens: usage["prompt_tokens_details"]["cached_tokens"]
                        .as_i64()
                        .unwrap_or_default(),
                    cache_creation_input_tokens:
                        usage["prompt_tokens_details"]["cache_write_tokens"]
                            .as_i64()
                            .unwrap_or_default(),
                };
                let choice = &body["choices"][0];
                let message = &choice["message"];
                let thinking = ["reasoning", "reasoning_content", "reasoning_text"]
                    .into_iter()
                    .filter_map(|k| message[k].as_str())
                    .find(|s| !s.trim().is_empty())
                    .unwrap_or_default()
                    .to_owned();
                let signature = if message["reasoning_details"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty())
                {
                    crate::wire::encode_reasoning_details(&message["reasoning_details"])
                } else {
                    text(message, "reasoning_opaque")
                };
                if !thinking.trim().is_empty() || !signature.is_empty() {
                    blocks.push(Block {
                        kind: "thinking".into(),
                        thinking,
                        signature,
                        ..Default::default()
                    });
                }
                let content = message["content"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        message["content"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|p| p["type"] == "text")
                            .filter_map(|p| p["text"].as_str().filter(|s| !s.is_empty()))
                            .collect::<Vec<_>>()
                            .join("\n")
                    });
                if !content.trim().is_empty() {
                    blocks.push(block_text(content, String::new()));
                }
                let refusal = text(message, "refusal");
                if !refusal.trim().is_empty() {
                    blocks.push(block_text(
                        format!("The model refused to respond: {}", refusal.trim()),
                        String::new(),
                    ));
                }
                let calls = message["tool_calls"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                for (index, call) in calls.iter().enumerate() {
                    let mut id = text(call, "id");
                    if id.is_empty() {
                        id = format!("call_{index}");
                    }
                    blocks.push(Block {
                        kind: "tool_use".into(),
                        id,
                        name: text(&call["function"], "name"),
                        input: Some(arguments(&call["function"]["arguments"])),
                        ..Default::default()
                    });
                }
                response.stop_reason = if choice["finish_reason"] == "length" {
                    "max_tokens"
                } else if !calls.is_empty() {
                    "tool_use"
                } else {
                    "end_turn"
                }
                .into();
            }
            Protocol::Anthropic => unreachable!(),
        }
        if !blocks.is_empty() {
            response.content = Some(blocks);
        }
        response
    };
    encode(&response)
}

pub(crate) fn stream_document<'a>(
    document: &JsonDocument,
    protocol: Protocol,
    stop_reason: Option<&str>,
    inputs: impl Iterator<Item = (usize, &'a Value, Option<&'a str>)>,
) -> Result<JsonDocument, Error> {
    let mut response: Response = serde_json::from_str(document.as_str()).map_err(|_| invalid())?;
    // The SDK StreamAssembler copies ID/model/role but does not copy Type.
    response.kind.clear();
    if let Some(reason) = stop_reason {
        response.stop_reason = reason.into();
    }
    if let Some(blocks) = &mut response.content {
        if protocol == Protocol::Anthropic {
            for (index, start, input) in inputs {
                let block = blocks.get_mut(index).ok_or_else(invalid)?;
                let mut original = std::mem::take(block);
                *block = match original.kind.as_str() {
                    "text" => block_text(
                        original
                            .text
                            .strip_prefix(start["text"].as_str().unwrap_or_default())
                            .unwrap_or(&original.text)
                            .into(),
                        String::new(),
                    ),
                    "thinking" => Block {
                        kind: original.kind,
                        thinking: original
                            .thinking
                            .strip_prefix(start["thinking"].as_str().unwrap_or_default())
                            .unwrap_or(&original.thinking)
                            .into(),
                        signature: original.signature,
                        ..Default::default()
                    },
                    "redacted_thinking" => Block {
                        kind: original.kind,
                        data: original.data,
                        ..Default::default()
                    },
                    "compaction" => Block {
                        kind: original.kind,
                        encrypted_content: original.encrypted_content,
                        content: original.content,
                        ..Default::default()
                    },
                    "tool_use" => Block {
                        kind: original.kind,
                        id: original.id,
                        name: original.name,
                        input: Some(
                            RawValue::from_string(input.unwrap_or("{}").to_owned())
                                .map_err(|_| invalid())?,
                        ),
                        ..Default::default()
                    },
                    _ => std::mem::take(&mut original),
                };
            }
        } else if protocol == Protocol::Responses {
            // The pinned Responses translator never closes compaction blocks;
            // StreamAssembler emits only blocks that received a stop event.
            blocks.retain(|block| block.kind != "compaction");
        }
        if blocks.is_empty() {
            response.content = None;
        }
    }
    encode(&response)
}

fn encode(response: &Response) -> Result<JsonDocument, Error> {
    let encoded = serde_json::to_string(response).map_err(|_| invalid())?;
    // Go compacts RawMessage fragments and HTML-escapes string content while
    // preserving number spellings, duplicate keys and typed member order.
    let mut compact = String::with_capacity(encoded.len());
    let mut quoted = false;
    let mut escaped = false;
    for c in encoded.chars() {
        if !quoted && c.is_ascii_whitespace() {
            continue;
        }
        match c {
            '<' => compact.push_str("\\u003c"),
            '>' => compact.push_str("\\u003e"),
            '&' => compact.push_str("\\u0026"),
            '\u{2028}' => compact.push_str("\\u2028"),
            '\u{2029}' => compact.push_str("\\u2029"),
            _ => compact.push(c),
        }
        if escaped {
            escaped = false;
        } else if quoted && c == '\\' {
            escaped = true;
        } else if c == '"' {
            quoted = !quoted;
        }
    }
    JsonDocument::new(compact).map_err(|_| invalid())
}
