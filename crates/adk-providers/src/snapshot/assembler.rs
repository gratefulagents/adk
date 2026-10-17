use super::{Block, Response, Usage, block_text, encode, invalid, null_default};
use adk_core::{Error, JsonDocument};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::BTreeMap;

/// Incremental SDK-shaped response assembly, independent of transport validation.
///
/// Only stopped blocks appear in snapshots, in stop order. Missing message starts,
/// repeated stops, replacement starts and unknown events are accepted as in the
/// pinned SDK assembler. This does not relax [`crate::client::StreamState`].
#[derive(Default)]
pub struct StreamAssembler {
    response: Response,
    builders: BTreeMap<i64, Builder>,
    completed: Vec<Builder>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Event {
    #[serde(rename = "type", deserialize_with = "null_default")]
    kind: String,
    #[serde(deserialize_with = "null_default")]
    index: i64,
    message: Option<Response>,
    content_block: Option<Block>,
    delta: Option<Delta>,
    usage: Option<Usage>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Delta {
    #[serde(rename = "type", deserialize_with = "null_default")]
    kind: String,
    #[serde(deserialize_with = "null_default")]
    text: String,
    #[serde(deserialize_with = "null_default")]
    thinking: String,
    #[serde(deserialize_with = "null_default")]
    signature: String,
    #[serde(deserialize_with = "null_default")]
    content: String,
    #[serde(deserialize_with = "null_default")]
    encrypted_content: String,
    #[serde(deserialize_with = "null_default")]
    partial_json: String,
    #[serde(deserialize_with = "null_default")]
    stop_reason: String,
    end_turn: Option<bool>,
}

struct Builder {
    block: Block,
    input: String,
}

impl StreamAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one JSON event. JSON syntax/type errors leave the assembler unchanged.
    /// This is a UTF-8 Serde input boundary, not a Go JSON decoder emulation.
    pub fn add(&mut self, event: &str) -> Result<(), Error> {
        let event: Event = serde_json::from_str(event).map_err(|_| invalid())?;
        match event.kind.as_str() {
            "message_start" => {
                if let Some(message) = event.message {
                    self.response.id = message.id;
                    self.response.model = message.model;
                    self.response.role = message.role;
                    self.response.usage = message.usage;
                }
            }
            "content_block_start" => {
                if let Some(block) = event.content_block {
                    self.builders.insert(
                        event.index,
                        Builder {
                            block: Block {
                                kind: block.kind,
                                id: block.id,
                                name: block.name,
                                phase: block.phase,
                                signature: block.signature,
                                data: block.data,
                                content: block.content,
                                created_by: block.created_by,
                                encrypted_content: block.encrypted_content,
                                ..Default::default()
                            },
                            input: String::new(),
                        },
                    );
                }
            }
            "content_block_delta" => {
                if let (Some(builder), Some(delta)) =
                    (self.builders.get_mut(&event.index), event.delta)
                {
                    match delta.kind.as_str() {
                        "text_delta" => builder.block.text.push_str(&delta.text),
                        "thinking_delta" => builder.block.thinking.push_str(&delta.thinking),
                        "signature_delta" => builder.block.signature.push_str(&delta.signature),
                        "reasoning_encrypted_content" | "compaction_encrypted_content" => {
                            builder.block.encrypted_content = delta.encrypted_content;
                        }
                        "input_json_delta" => builder.input.push_str(&delta.partial_json),
                        "compaction_delta" => {
                            builder.block.content.push_str(&delta.content);
                            if !delta.encrypted_content.is_empty() {
                                builder.block.encrypted_content = delta.encrypted_content;
                            }
                        }
                        _ => {}
                    }
                }
            }
            "content_block_stop" => {
                if let Some(builder) = self.builders.remove(&event.index) {
                    self.completed.push(builder);
                }
            }
            "message_delta" => {
                if let Some(delta) = event.delta {
                    if !delta.stop_reason.is_empty() {
                        self.response.stop_reason = delta.stop_reason;
                    }
                    if delta.end_turn.is_some() {
                        self.response.end_turn = delta.end_turn;
                    }
                }
                if let Some(usage) = event.usage {
                    for (target, value) in [
                        (&mut self.response.usage.input_tokens, usage.input_tokens),
                        (&mut self.response.usage.output_tokens, usage.output_tokens),
                        (
                            &mut self.response.usage.cache_read_input_tokens,
                            usage.cache_read_input_tokens,
                        ),
                        (
                            &mut self.response.usage.cache_creation_input_tokens,
                            usage.cache_creation_input_tokens,
                        ),
                    ] {
                        if value != 0 {
                            *target = value;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Return an owned snapshot without closing the assembler. Invalid accumulated
    /// tool JSON is reported here, matching failure to encode the SDK response.
    pub fn response(&self) -> Result<JsonDocument, Error> {
        let mut response = self.response.clone();
        for builder in &self.completed {
            let original = &builder.block;
            let block = match original.kind.as_str() {
                "text" => block_text(original.text.clone(), original.phase.clone()),
                "thinking" => Block {
                    kind: original.kind.clone(),
                    thinking: original.thinking.clone(),
                    id: original.id.clone(),
                    signature: original.signature.clone(),
                    encrypted_content: original.encrypted_content.clone(),
                    ..Default::default()
                },
                "redacted_thinking" => Block {
                    kind: original.kind.clone(),
                    data: original.data.clone(),
                    id: original.id.clone(),
                    encrypted_content: original.encrypted_content.clone(),
                    ..Default::default()
                },
                "compaction" => Block {
                    kind: original.kind.clone(),
                    id: original.id.clone(),
                    content: original.content.clone(),
                    encrypted_content: original.encrypted_content.clone(),
                    created_by: original.created_by.clone(),
                    ..Default::default()
                },
                "tool_use" => Block {
                    kind: original.kind.clone(),
                    id: original.id.clone(),
                    name: original.name.clone(),
                    input: Some(
                        RawValue::from_string(if builder.input.is_empty() {
                            "{}".into()
                        } else {
                            builder.input.clone()
                        })
                        .map_err(|_| invalid())?,
                    ),
                    ..Default::default()
                },
                _ => block_text(original.text.clone(), String::new()),
            };
            response.content.get_or_insert_with(Vec::new).push(block);
        }
        encode(&response)
    }
}
