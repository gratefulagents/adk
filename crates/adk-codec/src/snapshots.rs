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
