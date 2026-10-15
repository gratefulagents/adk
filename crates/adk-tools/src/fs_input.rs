use adk_core::ToolCall;
use serde::{Deserialize, de::MapAccess};
use serde_json::{Map, Value};

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    String,
    Bool,
}

struct Fields(Vec<(String, Value)>);
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

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub(crate) fn decode(
    call: &ToolCall,
    structure: &str,
    expected: &[(&str, Kind)],
) -> Result<Value, String> {
    call.validate_argument_projection()
        .map_err(|error| format!("Invalid input: {error}"))?;
    let raw = call.argument_text();
    if call.arguments.is_null() {
        serde_json::from_str::<Value>(&raw).map_err(|error| {
            if error.is_eof() {
                "Invalid input: unexpected end of JSON input".into()
            } else {
                format!("Invalid input: {error}")
            }
        })?;
        return Ok(Value::Object(Map::new()));
    }
    if !call.arguments.is_object() {
        return Err(format!(
            "Invalid input: json: cannot unmarshal {} into Go value of type fs.{structure}",
            kind(&call.arguments)
        ));
    }
    let Fields(fields) = serde_json::from_str(&raw).expect("validated JSON object");
    let mut output = Map::new();
    for (name, value) in fields {
        if value.is_null() {
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
            Kind::String => (value.is_string(), "string"),
            Kind::Bool => (value.is_boolean(), "bool"),
        };
        // Go retains the first type error even if a later alias has a valid value.
        if !valid {
            return Err(format!(
                "Invalid input: json: cannot unmarshal {} into Go struct field {structure}.{name} of type {expected}",
                kind(&value)
            ));
        }
        output.insert((*name).into(), value);
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
