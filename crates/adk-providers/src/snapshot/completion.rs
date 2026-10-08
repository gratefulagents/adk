//! Complete-call projections differ from the public streaming assembler.
use super::{Block, Response, compact_go_json, encode, invalid};
use crate::wire::Protocol;
use adk_core::{Error, JsonDocument};
use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use serde_json::{Value, value::RawValue};

pub(crate) fn document<'a>(
    document: &JsonDocument,
    protocol: Protocol,
    stop_reason: Option<&str>,
    usage: Option<&Value>,
    compactions: impl Iterator<Item = (usize, &'a str)>,
    inputs: impl Iterator<Item = (usize, &'a str)>,
) -> Result<JsonDocument, Error> {
    let mut response: Response = serde_json::from_str(document.as_str()).map_err(|_| invalid())?;
    response.kind = "message".into();
    if let Some(reason) = stop_reason {
        response.stop_reason = reason.into();
    }
    if let Some(usage) = usage {
        response.usage = serde_json::from_value(usage.clone()).map_err(|_| invalid())?;
    }
    if protocol == Protocol::Anthropic {
        response.end_turn = None;
    }
    if let Some(blocks) = &mut response.content {
        for (index, content) in compactions {
            let block = blocks.get_mut(index).ok_or_else(invalid)?;
            block.content = content.into();
        }
        for (index, input) in inputs {
            blocks.get_mut(index).ok_or_else(invalid)?.input =
                Some(RawValue::from_string(input.into()).map_err(|_| invalid())?);
        }
        for block in blocks {
            let original = std::mem::take(block);
            *block = match (protocol, original.kind.as_str()) {
                (Protocol::Anthropic, "text") => Block {
                    kind: original.kind,
                    text: original.text,
                    ..Default::default()
                },
                (Protocol::Anthropic, "thinking") => Block {
                    kind: original.kind,
                    thinking: original.thinking,
                    signature: original.signature,
                    ..Default::default()
                },
                (Protocol::Anthropic, "redacted_thinking") => Block {
                    kind: original.kind,
                    data: original.data,
                    ..Default::default()
                },
                (Protocol::Anthropic, "tool_use") => Block {
                    kind: original.kind,
                    id: original.id,
                    name: original.name,
                    input: original.input.map(canonical_input).transpose()?,
                    ..Default::default()
                },
                (Protocol::Anthropic, "compaction") => Block {
                    kind: original.kind,
                    encrypted_content: original.encrypted_content,
                    content: compaction_content(&original.content)?,
                    ..Default::default()
                },
                (Protocol::Responses, "text") => Block {
                    phase: String::new(),
                    ..original
                },
                (Protocol::Responses, "compaction") => Block {
                    created_by: String::new(),
                    content: String::new(),
                    ..original
                },
                _ => original,
            };
        }
    }
    encode(&response)
}

fn compaction_content(content: &str) -> Result<String, Error> {
    // Observed encoding of the pinned anthropic-sdk-go v1.38.0 BetaContentBlock
    // union after BetaMessage.Accumulate re-marshals a block. Kept only in the
    // compatibility document; the native compaction summary remains plain text.
    let text = compact_go_json(&serde_json::to_string(content).map_err(|_| invalid())?);
    Ok(include_str!("../anthropic_compaction_union.json")
        .trim_end()
        .replacen("\"__ADK_CONTENT__\"", &text, 1))
}

fn canonical_input(input: Box<RawValue>) -> Result<Box<RawValue>, Error> {
    let mut bytes = Vec::new();
    CanonicalInput(&input)
        .serialize(&mut serde_json::Serializer::with_formatter(
            &mut bytes, GoFloats,
        ))
        .map_err(|_| invalid())?;
    let text = String::from_utf8(bytes).map_err(|_| invalid())?;
    RawValue::from_string(compact_go_json(&text)).map_err(|_| invalid())
}

struct CanonicalInput<'a>(&'a RawValue);
impl Serialize for CanonicalInput<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let text = self.0.get().trim();
        match text.as_bytes().first() {
            Some(b'{') => {
                let values: std::collections::BTreeMap<String, Box<RawValue>> =
                    serde_json::from_str(text).map_err(serde::ser::Error::custom)?;
                let mut map = serializer.serialize_map(Some(values.len()))?;
                for (key, value) in values {
                    map.serialize_entry(&key, &CanonicalInput(&value))?;
                }
                map.end()
            }
            Some(b'[') => {
                let values: Vec<Box<RawValue>> =
                    serde_json::from_str(text).map_err(serde::ser::Error::custom)?;
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    sequence.serialize_element(&CanonicalInput(&value))?;
                }
                sequence.end()
            }
            Some(b'-' | b'0'..=b'9') => {
                let value: f64 = text.parse().map_err(serde::ser::Error::custom)?;
                if !value.is_finite() {
                    return Err(serde::ser::Error::custom(
                        "tool argument exceeds SDK Float64 range",
                    ));
                }
                serializer.serialize_f64(value)
            }
            _ => self.0.serialize(serializer),
        }
    }
}
struct GoFloats;
impl serde_json::ser::Formatter for GoFloats {
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
}
