use crate::dto::*;
use crate::go_decode::{ElementSeed, GoSlice, fold_key, go_object, normalize_strings, read_field};
use serde::Deserialize;

mod sealed {
    pub trait Sealed: serde::Serialize {
        const RAW_INPUT: bool = false;
        fn restore_raw(&mut self, _raw: super::RawJson) {}
        fn encode_bytes(&self) -> Result<Vec<u8>, serde_json::Error>
        where
            Self: Sized,
        {
            crate::snapshots::to_go_json(self)
        }
    }
}

/// The bounded compatibility records supported by [`from_go_json`].
///
/// This trait is sealed; `RunItem` and `Agent` are not supported targets.
pub trait GoJsonDecode: sealed::Sealed + serde::de::DeserializeOwned {}

/// Decode a pinned SDK record with Go string replacement and object semantics.
///
/// Unlike ordinary Serde JSON decoding, malformed UTF-8 and lone UTF-16
/// surrogates in typed strings and keys are replaced as in Go. Raw `input`
/// values retain their original lexical JSON, including duplicate keys and
/// escapes and invalid UTF-8 bytes. Only a validation copy is normalized; raw
/// bytes are restored after the entire decode succeeds. No `Value` round trip
/// is used.
/// The Go nesting limit of 10,000 is enforced, including ignored and raw values.
/// Errors do not expose Go's partial mutation state or reproduce its error text.
/// Existing `Vec` image fields collapse Go's nil and empty slices; both are
/// omitted on the wire. Slot reuse is retained during each object decode.
///
/// ```compile_fail
/// let _: adk_codec::dto::RunItem = adk_codec::from_go_json(b"{}").unwrap();
/// ```
pub fn from_go_json<T: GoJsonDecode>(input: &[u8]) -> Result<T, serde_json::Error> {
    check_depth(input)?;
    let (bytes, raw) = if T::RAW_INPUT {
        normalize_payload(input)?
    } else {
        (normalize_strings(input), None)
    };
    let mut decoded: T = serde_json::from_slice(&bytes)?;
    if let Some(raw) = raw {
        decoded.restore_raw(match std::str::from_utf8(raw) {
            Ok(text) => {
                RawJson::Encoded(serde_json::value::RawValue::from_string(text.to_owned())?)
            }
            Err(_) => RawJson::EncodedBytes(raw.to_vec()),
        });
    }
    Ok(decoded)
}

/// The bounded compatibility records supported by [`to_go_json_bytes`].
///
/// This trait is sealed; `RunItem` and `Agent` are not supported targets.
pub trait GoJsonEncode: sealed::Sealed {}

/// Encode a record with Go field order, raw-byte preservation and HTML escaping.
///
/// Unlike ordinary Serde serialization, raw input may contain invalid UTF-8.
/// Public `EncodedBytes` values are syntax-checked without a `Value` projection.
///
/// ```compile_fail
/// adk_codec::to_go_json_bytes(&adk_codec::dto::RunItem::default()).unwrap();
/// ```
pub fn to_go_json_bytes<T: GoJsonEncode>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    value.encode_bytes()
}

pub(crate) fn check_depth(input: &[u8]) -> Result<(), serde_json::Error> {
    let mut depth = 0usize;
    let mut i = 0;
    while i < input.len() {
        match input[i] {
            b'"' => {
                i = string_end(input, i);
                continue;
            }
            b'{' | b'[' => {
                depth += 1;
                if depth > 10_000 {
                    return Err(serde::de::Error::custom("exceeded Go JSON maximum depth"));
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    Ok(())
}

fn append_raw(bytes: &mut Vec<u8>, raw: &RawJson) -> Result<(), serde_json::Error> {
    // Public RawValue construction does not enforce Go's nesting limit.
    // Apply the same limit regardless of the chosen raw storage.
    check_depth(&raw.bytes())?;
    if let RawJson::EncodedBytes(raw_bytes) = raw {
        let normalized = normalize_strings(raw_bytes);
        let _: &serde_json::value::RawValue = serde_json::from_slice(&normalized)?;
        crate::snapshots::compact_raw_bytes(bytes, raw_bytes).map_err(serde_json::Error::io)?;
    } else {
        bytes.extend(crate::snapshots::to_go_json(raw)?);
    }
    Ok(())
}

macro_rules! target {
    ($($ty:ident),* $(,)?) => {$(
        impl sealed::Sealed for $ty {}
        impl GoJsonDecode for $ty {}
        impl GoJsonEncode for $ty {}
    )*};
}
target!(
    ModeRoleRouting,
    ModeRouting,
    ModeConstraints,
    ModeTemplateSpec,
    RoleSpec,
    RoleCatalog,
    Cursor,
    Usage,
    MessageOutput,
    ImageAttachment,
    ToolOutputData,
    HandoffCallData,
    HandoffOutputData,
    ReasoningData,
    CompactionData,
    RunItemType
);
impl sealed::Sealed for ToolCallData {
    const RAW_INPUT: bool = true;
    fn restore_raw(&mut self, raw: RawJson) {
        self.input = raw;
    }
    fn encode_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut bytes = b"{\"id\":".to_vec();
        bytes.extend(crate::snapshots::to_go_json(&self.id)?);
        bytes.extend_from_slice(b",\"name\":");
        bytes.extend(crate::snapshots::to_go_json(&self.name)?);
        bytes.extend_from_slice(b",\"input\":");
        append_raw(&mut bytes, &self.input)?;
        bytes.push(b'}');
        Ok(bytes)
    }
}
impl GoJsonDecode for ToolCallData {}
impl GoJsonEncode for ToolCallData {}
impl sealed::Sealed for ToolApprovalData {
    const RAW_INPUT: bool = true;
    fn restore_raw(&mut self, raw: RawJson) {
        self.input = raw;
    }
    fn encode_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut bytes = b"{\"tool_name\":".to_vec();
        bytes.extend(crate::snapshots::to_go_json(&self.tool_name)?);
        bytes.extend_from_slice(b",\"input\":");
        append_raw(&mut bytes, &self.input)?;
        bytes.extend_from_slice(b",\"call_id\":");
        bytes.extend(crate::snapshots::to_go_json(&self.call_id)?);
        bytes.extend_from_slice(b",\"approved\":");
        bytes.extend(crate::snapshots::to_go_json(&self.approved)?);
        bytes.push(b'}');
        Ok(bytes)
    }
}
impl GoJsonDecode for ToolApprovalData {}
impl GoJsonEncode for ToolApprovalData {}

go_object!(ModeRoleRouting,
    "model" => model: scalar,
    "reasoninglevel" => reasoning_level: scalar,
    "textverbosity" => text_verbosity: scalar
    preserve_nil { "fallbackmodels" => fallback_models }
);
go_object!(ModeRouting,
    "defaultmodel" => default_model: scalar,
    "reasoninglevel" => reasoning_level: scalar,
    "textverbosity" => text_verbosity: scalar,
    "roleoverrides" => role_overrides: map
    preserve_nil { "fallbackmodels" => fallback_models }
);
go_object!(ModeConstraints,
    "maxturns" => max_turns: integer,
    "subagentmaxturns" => sub_agent_max_turns: integer,
    "maxconcurrentsubagents" => max_concurrent_sub_agents: integer,
    "maxretries" => max_retries: integer,
    "maxruntimeminutes" => max_runtime_minutes: integer
);
go_object!(ModeTemplateSpec,
    "name" => name: scalar,
    "version" => version: scalar,
    "displayname" => display_name: scalar,
    "description" => description: scalar,
    "category" => category: scalar,
    "autonomous" => autonomous: scalar,
    "toolaccess" => tool_access: scalar,
    "instructions" => instructions: scalar,
    pointers {
        "modelrouting" => model_routing,
        "constraints" => constraints
    }
);

go_object!(RoleSpec,
    "name" => name: scalar,
    "description" => description: scalar,
    "instructions" => instructions: scalar,
    "toolaccess" => tool_access: scalar,
    "modeloverride" => model_override: scalar
    preserve_nil { "fallbackmodels" => fallback_models }
);

impl<'de> Deserialize<'de> for RoleCatalog {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self(GoSlice::<RoleSpec>::deserialize(d)?.into_values()))
    }

    fn deserialize_in_place<D: serde::Deserializer<'de>>(
        d: D,
        out: &mut Self,
    ) -> Result<(), D::Error> {
        let mut roles = out.0.take().map(GoSlice::from_values).unwrap_or_default();
        GoSlice::deserialize_in_place(d, &mut roles)?;
        out.0 = roles.into_values();
        Ok(())
    }
}

go_object!(Cursor, "message_id" => message_id: integer, "token" => token: scalar);
go_object!(Usage, "requests" => requests: integer, "input_tokens" => input_tokens: integer, "output_tokens" => output_tokens: integer, "cache_read_tokens" => cache_read_tokens: integer, "cache_create_tokens" => cache_create_tokens: integer);
go_object!(MessageOutput, "text" => text: scalar, "phase" => phase: scalar; "images" => images);
go_object!(ImageAttachment, "media_type" => media_type: scalar, "data" => data: scalar, "detail" => detail: scalar);
go_object!(ToolCallData, "id" => id: scalar, "name" => name: scalar, "input" => input: raw);
go_object!(ToolOutputData, "call_id" => call_id: scalar, "content" => content: scalar, "is_error" => is_error: scalar; "images" => images);
go_object!(HandoffCallData, "from_agent" => from_agent: scalar, "to_agent" => to_agent: scalar);
go_object!(HandoffOutputData, "from_agent" => from_agent: scalar, "to_agent" => to_agent: scalar);
go_object!(ReasoningData, "id" => id: scalar, "text" => text: scalar, "signature" => signature: scalar, "redacted_data" => redacted_data: scalar, "encrypted_content" => encrypted_content: scalar);
go_object!(CompactionData, "id" => id: scalar, "content" => content: scalar, "encrypted_content" => encrypted_content: scalar, "created_by" => created_by: scalar);
go_object!(ToolApprovalData, "tool_name" => tool_name: scalar, "input" => input: raw, "call_id" => call_id: scalar, "approved" => approved: scalar);

impl<'de> Deserialize<'de> for RunItemType {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut out = Self::default();
        Self::deserialize_in_place(d, &mut out)?;
        Ok(out)
    }
    fn deserialize_in_place<D: serde::Deserializer<'de>>(
        d: D,
        out: &mut Self,
    ) -> Result<(), D::Error> {
        let raw = Box::<serde_json::value::RawValue>::deserialize(d)?;
        if raw.get() != "null" {
            out.0 = raw.get().parse::<i64>().map_err(serde::de::Error::custom)?;
        }
        Ok(())
    }
}

// Scan only to protect raw spans. The JSON decoder remains the syntax authority.
fn string_end(input: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < input.len() {
        match input[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    input.len()
}

fn raw_end(input: &[u8], start: usize) -> usize {
    let mut i = start;
    let mut depth = 0usize;
    while i < input.len() {
        match input[i] {
            b'"' => {
                i = string_end(input, i);
                continue;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' if depth > 0 => depth -= 1,
            b'}' | b']' | b',' if depth == 0 => break,
            b' ' | b'\t' | b'\r' | b'\n' if depth == 0 => break,
            _ => {}
        }
        i += 1;
    }
    i
}

fn normalize_payload(input: &[u8]) -> Result<(Vec<u8>, Option<&[u8]>), serde_json::Error> {
    let mut last_raw = None;
    let mut bytes = Vec::with_capacity(input.len());
    let mut depth = 0usize;
    let mut i = 0;
    let mut raw_field = false;
    while i < input.len() {
        match input[i] {
            b'"' => {
                let end = string_end(input, i);
                let normalized = normalize_strings(&input[i..end]);
                if depth == 1 {
                    raw_field =
                        fold_key(&serde_json::from_slice::<String>(&normalized)?) == "input";
                }
                bytes.extend_from_slice(&normalized);
                i = end;
                continue;
            }
            b':' if depth == 1 && raw_field => {
                bytes.push(b':');
                i += 1;
                while input.get(i).is_some_and(u8::is_ascii_whitespace) {
                    bytes.push(input[i]);
                    i += 1;
                }
                let end = raw_end(input, i);
                last_raw = Some(&input[i..end]);
                bytes.extend(normalize_strings(&input[i..end]));
                i = end;
                raw_field = false;
                continue;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        bytes.push(input[i]);
        i += 1;
    }
    Ok((bytes, last_raw))
}
