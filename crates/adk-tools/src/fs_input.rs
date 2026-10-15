use adk_core::ToolCall;
use serde::{Deserialize, de::MapAccess};
use serde_json::{Map, Value, value::RawValue};

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    String,
    Bool,
}

struct Fields(Vec<(String, Box<RawValue>)>);
impl<'de> Deserialize<'de> for Fields {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Fields;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a filesystem tool input object")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Fields, M::Error> {
                let mut fields = Vec::new();
                while let Some(field) = map.next_entry()? {
                    fields.push(field);
                }
                Ok(Fields(fields))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

fn kind(value: &RawValue) -> &'static str {
    match value.get().as_bytes()[0] {
        b'n' => "null",
        b't' | b'f' => "bool",
        b'"' => "string",
        b'[' => "array",
        b'{' => "object",
        _ => "number",
    }
}

pub(crate) fn decode(
    call: &ToolCall,
    structure: &str,
    expected: &[(&str, Kind)],
) -> Result<Value, String> {
    call.validate_argument_projection()
        .map_err(|error| format!("Invalid input: {error}"))?;
    let mut raw = call.argument_text().as_bytes().to_vec();
    let mut quoted = false;
    let mut depth = 0usize;
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'"' {
            quoted = !quoted;
        } else if quoted && raw[i] == b'\\' {
            if raw.get(i + 1) == Some(&b'u') {
                let hex = |start: usize| {
                    raw.get(start..start + 4)
                        .and_then(|digits| std::str::from_utf8(digits).ok())
                        .and_then(|digits| u16::from_str_radix(digits, 16).ok())
                };
                if let Some(code) = hex(i + 2) {
                    if (0xd800..=0xdbff).contains(&code)
                        && raw.get(i + 6..i + 8) == Some(b"\\u")
                        && hex(i + 8).is_some_and(|low| (0xdc00..=0xdfff).contains(&low))
                    {
                        i += 12;
                        continue;
                    }
                    // Go replaces lone UTF-16 surrogates rather than rejecting JSON strings.
                    if (0xd800..=0xdfff).contains(&code) {
                        raw[i + 2..i + 6].copy_from_slice(b"fffd");
                    }
                    i += 6;
                    continue;
                }
            }
            i += 2;
            continue;
        } else if !quoted {
            match raw[i] {
                b'[' | b'{' => {
                    depth += 1;
                    if depth > 10000 {
                        return Err(format!(
                            "Invalid input: invalid character '{}' exceeded max depth",
                            raw[i] as char
                        ));
                    }
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        i += 1;
    }
    let arguments = serde_json::from_slice::<Box<RawValue>>(&raw).map_err(|error| {
        if error.is_eof() {
            "Invalid input: unexpected end of JSON input".into()
        } else {
            format!("Invalid input: {error}")
        }
    })?;
    if kind(&arguments) == "null" {
        return Ok(Value::Object(Map::new()));
    }
    if kind(&arguments) != "object" {
        return Err(format!(
            "Invalid input: json: cannot unmarshal {} into Go value of type fs.{structure}",
            kind(&arguments)
        ));
    }
    let Fields(fields) = serde_json::from_slice(&raw).expect("validated JSON object");
    let mut output = Map::new();
    for (name, value) in fields {
        if kind(&value) == "null" {
            continue;
        }
        let folded: String = name
            .chars()
            .map(|c| match c {
                'ſ' => 's',
                'K' => 'k',
                _ => c.to_ascii_lowercase(),
            })
            .collect();
        let Some((name, expected)) = expected.iter().find(|(name, _)| *name == folded) else {
            continue;
        };
        let (valid, expected) = match expected {
            Kind::String => (kind(&value) == "string", "string"),
            Kind::Bool => (kind(&value) == "bool", "bool"),
        };
        // Go retains the first type error even if a later alias has a valid value.
        if !valid {
            return Err(format!(
                "Invalid input: json: cannot unmarshal {} into Go struct field {structure}.{name} of type {expected}",
                kind(&value)
            ));
        }
        output.insert(
            (*name).into(),
            serde_json::from_str(value.get()).expect("validated scalar"),
        );
    }
    Ok(Value::Object(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(raw: &str) -> ToolCall {
        ToolCall {
            id: "input".into(),
            name: "Edit".into(),
            arguments: serde_json::from_str(raw).unwrap_or(Value::Null),
            raw_arguments: Some(raw.into()),
        }
    }

    #[test]
    fn ordered_aliases_and_nulls_preserve_go_field_updates() {
        let fields = &[
            ("file_path", Kind::String),
            ("old_string", Kind::String),
            ("new_string", Kind::String),
            ("replace_all", Kind::Bool),
        ];
        let raw = r#"{"FILE_PATH":"wrong","file_path":"a","old_ſtring":"alpha","new_string":"first","NEW_STRING":"second","new_string":null,"replace_all":true,"REPLACE_ALL":null}"#;
        assert_eq!(
            decode(&call(raw), "fileEditInput", fields).unwrap(),
            json!({"file_path":"a","old_string":"alpha","new_string":"second","replace_all":true})
        );
        let raw = r#"{"file_path":"a","FILE_PATH":"b"}"#;
        assert_eq!(
            decode(&call(raw), "fileEditInput", fields).unwrap(),
            json!({"file_path":"b"})
        );
        let mut native = call(r#"{"FILE_PATH":"a","OLD_STRING":"alpha"}"#);
        native.raw_arguments = None;
        assert_eq!(
            decode(&native, "fileEditInput", fields).unwrap(),
            json!({"file_path":"a","old_string":"alpha"})
        );
    }

    #[test]
    fn earlier_type_errors_are_not_erased_by_later_valid_duplicates() {
        for raw in [
            r#"{"content":false,"content":"valid"}"#,
            r#"{"CONTENT":false,"content":"valid"}"#,
            r#"{"content":false,"file_path":7}"#,
        ] {
            assert_eq!(
                decode(
                    &call(raw),
                    "fileWriteInput",
                    &[("content", Kind::String), ("file_path", Kind::String)]
                )
                .unwrap_err(),
                "Invalid input: json: cannot unmarshal bool into Go struct field fileWriteInput.content of type string"
            );
        }
    }

    #[test]
    fn unknown_operation_fields_do_not_participate_in_decoding() {
        let raw = r#"{"PATH":"a","source_path":{},"destination_path":false}"#;
        assert_eq!(
            decode(&call(raw), "deleteInput", &[("path", Kind::String)]).unwrap(),
            json!({"path":"a"})
        );
        let raw =
            r#"{"ſource_path":"a","deſtination_path":"b","path":[],"unknown":{"nested":null}}"#;
        assert_eq!(
            decode(
                &call(raw),
                "moveInput",
                &[
                    ("source_path", Kind::String),
                    ("destination_path", Kind::String)
                ]
            )
            .unwrap(),
            json!({"source_path":"a","destination_path":"b"})
        );
    }

    #[test]
    fn absent_raw_input_differs_from_null_and_mismatched_projections_fail() {
        assert_eq!(
            decode(&call(""), "fileWriteInput", &[]).unwrap_err(),
            "Invalid input: unexpected end of JSON input"
        );
        assert_eq!(
            decode(&call("null"), "fileWriteInput", &[]).unwrap(),
            json!({})
        );
        let mut mismatched = call(r#"{"file_path":"other"}"#);
        mismatched.arguments = json!({"file_path":"approved"});
        assert!(
            decode(
                &mismatched,
                "fileWriteInput",
                &[("file_path", Kind::String)]
            )
            .unwrap_err()
            .contains("raw tool arguments disagree")
        );
    }

    #[test]
    fn surrogate_replacement_preserves_pairs_escaped_text_and_field_order() {
        for (encoded, expected) in [
            (r"\ud800", "�"),
            (r"\udfff", "�"),
            (r"\ud800\ud800", "��"),
            (r"\ud83d\ude00", "😀"),
            (r"\\ud800", r"\ud800"),
            (r"\ud800\u0061", "�a"),
            (r"\ud800\udc00\ud800", "𐀀�"),
        ] {
            let raw = format!(r#"{{"content":"first","CONTENT":"{encoded}","content":null}}"#);
            assert_eq!(
                decode(&call(&raw), "fileWriteInput", &[("content", Kind::String)]).unwrap(),
                json!({"content":expected})
            );
        }
        for raw in [r#"{"content":"\ud80"}"#, r#"{"content":"\ud80x"}"#] {
            assert!(decode(&call(raw), "fileWriteInput", &[("content", Kind::String)]).is_err());
        }
    }

    #[test]
    fn ignored_fields_use_go_depth_limit_without_recursive_value_decoding() {
        for depth in [127, 128, 150, 9999] {
            let raw = format!(
                r#"{{"content":"ok","unknown":{}0{}}}"#,
                "[".repeat(depth),
                "]".repeat(depth)
            );
            assert_eq!(
                decode(&call(&raw), "fileWriteInput", &[("content", Kind::String)]).unwrap(),
                json!({"content":"ok"})
            );
        }
        let raw = format!(
            r#"{{"content":"ok","unknown":{}0{}}}"#,
            "[".repeat(10000),
            "]".repeat(10000)
        );
        assert_eq!(
            decode(&call(&raw), "fileWriteInput", &[("content", Kind::String)]).unwrap_err(),
            "Invalid input: invalid character '[' exceeded max depth"
        );
        let raw = format!(r#"{{"content":{}0{}}}"#, "[".repeat(150), "]".repeat(150));
        assert_eq!(
            decode(&call(&raw), "fileWriteInput", &[("content", Kind::String)]).unwrap_err(),
            "Invalid input: json: cannot unmarshal array into Go struct field fileWriteInput.content of type string"
        );
        let brackets = "[".repeat(10001);
        let raw = format!(r#"{{"content":"{brackets}"}}"#);
        assert_eq!(
            decode(&call(&raw), "fileWriteInput", &[("content", Kind::String)]).unwrap(),
            json!({"content":brackets})
        );
    }

    #[test]
    fn scalar_roots_retain_go_type_errors() {
        for (raw, kind) in [
            ("[]", "array"),
            ("true", "bool"),
            ("123", "number"),
            (r#""text""#, "string"),
        ] {
            assert_eq!(
                decode(&call(raw), "fileWriteInput", &[]).unwrap_err(),
                format!(
                    "Invalid input: json: cannot unmarshal {kind} into Go value of type fs.fileWriteInput"
                )
            );
        }
    }
}
