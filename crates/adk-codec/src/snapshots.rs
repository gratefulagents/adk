//! Analysis-oriented request documents; executable tool callbacks never serialize.

use crate::dto::{RawJson, RunItemSnapshot, null_default};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "null_default")]
    #[serde(serialize_with = "finite_optional")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "is_zero")]
    #[serde(deserialize_with = "null_default")]
    pub max_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "null_default")]
    #[serde(serialize_with = "finite_optional")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub tool_choice: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "null_default")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "is_zero")]
    #[serde(deserialize_with = "null_default")]
    pub thinking_budget: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub reasoning_effort: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub text_verbosity: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub stop_sequences: Vec<String>,
}
fn is_zero(value: &i64) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RequestSnapshot {
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub agent_name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub instructions: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub input_items: Vec<RunItemSnapshot>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub tools: Vec<ToolSnapshot>,
    #[serde(deserialize_with = "null_default")]
    pub settings: ModelSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "null_default")]
    pub output_schema: Option<OutputSchema>,
    #[serde(skip_serializing_if = "is_zero")]
    #[serde(deserialize_with = "null_default")]
    pub input_token_estimate: i64,
    #[serde(skip_serializing_if = "is_zero")]
    #[serde(deserialize_with = "null_default")]
    pub request_overhead_token_estimate: i64,
    #[serde(skip_serializing_if = "is_zero")]
    #[serde(deserialize_with = "null_default")]
    pub total_token_estimate: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolSnapshot {
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(deserialize_with = "null_default")]
    pub description: String,
    #[serde(skip_serializing_if = "RawJson::is_missing")]
    pub input_schema: RawJson,
    #[serde(deserialize_with = "null_default")]
    pub read_only: bool,
    #[serde(deserialize_with = "null_default")]
    pub needs_approval: bool,
    #[serde(skip_serializing_if = "is_zero")]
    #[serde(deserialize_with = "null_default")]
    pub timeout_seconds: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputSchema {
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(skip_serializing_if = "RawJson::is_missing")]
    pub schema: RawJson,
    #[serde(deserialize_with = "null_default")]
    pub strict: bool,
}

struct GoFormatter;
impl serde_json::ser::Formatter for GoFormatter {
    fn write_raw_fragment<W: std::io::Write + ?Sized>(
        &mut self,
        writer: &mut W,
        fragment: &str,
    ) -> std::io::Result<()> {
        let mut quoted = false;
        let mut escaped = false;
        for character in fragment.chars() {
            if !quoted && character.is_ascii_whitespace() {
                continue;
            }
            match character {
                '<' => writer.write_all(b"\\u003c")?,
                '>' => writer.write_all(b"\\u003e")?,
                '&' => writer.write_all(b"\\u0026")?,
                '\u{2028}' => writer.write_all(b"\\u2028")?,
                '\u{2029}' => writer.write_all(b"\\u2029")?,
                character => {
                    let mut bytes = [0; 4];
                    writer.write_all(character.encode_utf8(&mut bytes).as_bytes())?;
                }
            }
            if escaped {
                escaped = false;
            } else if quoted && character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = !quoted;
            }
        }
        Ok(())
    }

    fn write_f64<W: std::io::Write + ?Sized>(
        &mut self,
        writer: &mut W,
        value: f64,
    ) -> std::io::Result<()> {
        let magnitude = value.abs();
        let text = if magnitude != 0.0 && !(1e-6..1e21).contains(&magnitude) {
            let text = format!("{value:e}");
            let (mantissa, exponent) = text.split_once('e').expect("scientific float");
            if exponent.starts_with('-') {
                text
            } else {
                format!("{mantissa}e+{exponent}")
            }
        } else {
            value.to_string()
        };
        writer.write_all(text.as_bytes())
    }
    fn write_f32<W: std::io::Write + ?Sized>(
        &mut self,
        writer: &mut W,
        value: f32,
    ) -> std::io::Result<()> {
        let magnitude = value.abs();
        let text = if magnitude != 0.0 && !(1e-6..1e21).contains(&magnitude) {
            let text = format!("{value:e}");
            let (mantissa, exponent) = text.split_once('e').expect("scientific float");
            if exponent.starts_with('-') {
                text
            } else {
                format!("{mantissa}e+{exponent}")
            }
        } else {
            value.to_string()
        };
        writer.write_all(text.as_bytes())
    }
    fn write_string_fragment<W: std::io::Write + ?Sized>(
        &mut self,
        writer: &mut W,
        fragment: &str,
    ) -> std::io::Result<()> {
        for character in fragment.chars() {
            match character {
                '<' => writer.write_all(b"\\u003c")?,
                '>' => writer.write_all(b"\\u003e")?,
                '&' => writer.write_all(b"\\u0026")?,
                '\u{2028}' => writer.write_all(b"\\u2028")?,
                '\u{2029}' => writer.write_all(b"\\u2029")?,
                character => {
                    let mut bytes = [0; 4];
                    writer.write_all(character.encode_utf8(&mut bytes).as_bytes())?;
                }
            }
        }
        Ok(())
    }
}

/// Preserve field declaration order, Go float notation and HTML-safe strings.
pub fn to_go_json(value: &impl Serialize) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = Vec::new();
    value.serialize(&mut serde_json::Serializer::with_formatter(
        &mut bytes,
        GoFormatter,
    ))?;
    Ok(bytes)
}

fn finite_optional<S: serde::Serializer>(
    value: &Option<f64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if value.is_some_and(|number| !number.is_finite()) {
        return Err(serde::ser::Error::custom("non-finite model setting"));
    }
    value.serialize(serializer)
}

#[derive(Deserialize)]
struct NormalizedMessage {
    #[serde(rename = "id")]
    _id: String,
    #[serde(rename = "type")]
    _kind: String,
    #[serde(rename = "role")]
    _role: String,
    #[serde(deserialize_with = "Option::deserialize")]
    content: Option<Vec<NormalizedBlock>>,
    #[serde(rename = "model")]
    _model: String,
    #[serde(rename = "stop_reason")]
    _stop_reason: String,
    usage: NormalizedUsage,
    end_turn: Option<bool>,
}

#[derive(Deserialize)]
struct NormalizedUsage {
    input_tokens: i64,
    output_tokens: i64,
    #[serde(default)]
    cache_read_input_tokens: i64,
    #[serde(default)]
    cache_creation_input_tokens: i64,
}

#[derive(Deserialize)]
struct NormalizedBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    phase: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    input: RawJson,
    #[serde(default)]
    thinking: String,
    #[serde(default)]
    signature: String,
    #[serde(default)]
    data: String,
    #[serde(default)]
    encrypted_content: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    created_by: String,
}

fn projected_snapshot(
    raw: &adk_core::JsonDocument,
    projection: adk_core::SnapshotProjection,
) -> Result<crate::dto::ResponseSnapshot, crate::approval::BridgeError> {
    use crate::{approval::BridgeError, dto};
    let message: NormalizedMessage = serde_json::from_str(raw.as_str())
        .map_err(|_| BridgeError("invalid normalized provider snapshot"))?;
    let openai = matches!(projection, adk_core::SnapshotProjection::GoOpenAiMessage);
    let items = message
        .content
        .unwrap_or_default()
        .into_iter()
        .filter_map(|block| {
            let mut item = dto::RunItem::default();
            match block.kind.as_str() {
                "text" => {
                    item.message = Some(dto::MessageOutput {
                        text: block.text,
                        phase: if openai { block.phase } else { String::new() },
                        ..Default::default()
                    });
                }
                "tool_use" => {
                    item.kind = dto::RunItemType(1);
                    item.tool_call = Some(dto::ToolCallData {
                        id: block.id,
                        name: block.name,
                        input: block.input,
                    });
                }
                "thinking" | "redacted_thinking" => {
                    item.kind = dto::RunItemType(5);
                    let mut reasoning = dto::ReasoningData {
                        id: block.id,
                        encrypted_content: block.encrypted_content,
                        ..Default::default()
                    };
                    if block.kind == "thinking" {
                        reasoning.text = block.thinking;
                        reasoning.signature = block.signature;
                    } else {
                        reasoning.redacted_data = block.data;
                    }
                    item.reasoning = Some(reasoning);
                }
                "compaction" => {
                    item.kind = dto::RunItemType(7);
                    item.compaction = Some(dto::CompactionData {
                        id: block.id,
                        content: block.content,
                        encrypted_content: block.encrypted_content,
                        created_by: block.created_by,
                    });
                }
                _ => return None,
            }
            Some(item)
        })
        .collect();
    let wire = dto::ModelResponse {
        items: Some(items),
        usage: dto::Usage {
            requests: 1,
            input_tokens: message.usage.input_tokens,
            output_tokens: message.usage.output_tokens,
            cache_read_tokens: message.usage.cache_read_input_tokens,
            cache_create_tokens: message.usage.cache_creation_input_tokens,
        },
        end_turn: if openai { message.end_turn } else { None },
        ..Default::default()
    };
    let mut snapshot = crate::response_snapshot(&wire);
    snapshot.raw_available = true;
    snapshot.raw = RawJson::Encoded(
        serde_json::value::RawValue::from_string(raw.as_str().to_owned())
            .expect("validated JSON document"),
    );
    Ok(snapshot)
}

/// Converts representable native response items without inventing agent attribution.
/// Raw data retains its native shape. URI media, native handoffs, paused tool outputs
/// and usage counters above the SDK signed range return a bridge error.
impl TryFrom<&adk_core::ModelResponse> for crate::dto::ResponseSnapshot {
    type Error = crate::approval::BridgeError;

    fn try_from(response: &adk_core::ModelResponse) -> Result<Self, Self::Error> {
        use crate::{
            approval::{BridgeError, encode_content, encode_item},
            dto,
        };
        if let Some(projection) = response.snapshot_projection {
            let raw = response
                .snapshot_raw
                .as_ref()
                .ok_or(BridgeError("missing normalized provider snapshot"))?;
            return projected_snapshot(raw, projection);
        }
        let count = |value: u64| {
            i64::try_from(value).map_err(|_| BridgeError("usage counter exceeds SDK signed range"))
        };
        let items = response
            .items
            .iter()
            .map(|item| match item {
                adk_core::RunItem::Message { message }
                | adk_core::RunItem::PhasedMessage { message, .. } => {
                    let (text, images) = encode_content(&message.content)?;
                    Ok(dto::RunItem {
                        message: Some(dto::MessageOutput {
                            text,
                            images,
                            phase: match item {
                                adk_core::RunItem::PhasedMessage { phase, .. } => phase.clone(),
                                _ => String::new(),
                            },
                        }),
                        ..Default::default()
                    })
                }
                _ => encode_item(item, None),
            })
            .collect::<Result<Vec<_>, BridgeError>>()?;
        let wire = dto::ModelResponse {
            items: Some(items),
            usage: dto::Usage {
                requests: count(response.usage.requests)?,
                input_tokens: count(response.usage.input_tokens)?,
                output_tokens: count(response.usage.output_tokens)?,
                cache_read_tokens: count(response.usage.cache_read_tokens)?,
                cache_create_tokens: count(response.usage.cache_creation_tokens)?,
            },
            end_turn: response.end_turn,
            raw: response.raw.clone().unwrap_or(serde_json::Value::Null),
            ..Default::default()
        };
        let mut snapshot = crate::response_snapshot(&wire);
        if let Some(raw) = &response.snapshot_raw {
            snapshot.raw_available = true;
            snapshot.raw = RawJson::Encoded(
                serde_json::value::RawValue::from_string(raw.as_str().to_owned())
                    .expect("validated JSON document"),
            );
        } else if response
            .raw
            .as_ref()
            .is_some_and(serde_json::Value::is_null)
        {
            snapshot.raw_available = true;
            snapshot.raw = RawJson::Present(serde_json::Value::Null);
        }
        Ok(snapshot)
    }
}
